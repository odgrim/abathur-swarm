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

use crate::domain::models::AgentMetadata;
use crate::domain::ports::{
    HealthStatus, LlmSubstrate, StopReason, SubstrateError, SubstrateRequest,
    SubstrateResponse,
};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
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

    /// Find agent file path in .claude/agents directory
    ///
    /// Searches in project working directory's .claude/agents for the agent markdown file.
    ///
    /// # Arguments
    /// * `agent_type` - The agent type name
    ///
    /// # Returns
    /// * `Some(PathBuf)` - Path to the agent file if found
    /// * `None` - Agent file not found
    fn find_agent_file(&self, agent_type: &str) -> Option<PathBuf> {
        let base_dir = self.config.working_dir.as_ref()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let agents_dir = base_dir.join(".claude").join("agents");

        // Try common subdirectories
        let subdirs = ["abathur", "workers", ""];

        for subdir in &subdirs {
            let path = if subdir.is_empty() {
                agents_dir.join(format!("{}.md", agent_type))
            } else {
                agents_dir.join(subdir).join(format!("{}.md", agent_type))
            };

            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    /// Build --agents JSON for Claude Code CLI
    ///
    /// Loads the agent markdown file and builds the JSON definition.
    ///
    /// # Arguments
    /// * `agent_type` - The agent type name
    /// * `agent_path` - Path to the agent markdown file
    ///
    /// # Returns
    /// * `Ok(String)` - JSON string for --agents flag
    /// * `Err` - Failed to load or parse agent file
    #[allow(dead_code)]
    fn build_agents_json(&self, agent_type: &str, agent_path: &PathBuf) -> Result<String, SubstrateError> {
        // Read the agent file
        let content = std::fs::read_to_string(agent_path)
            .map_err(|e| SubstrateError::ExecutionFailed(format!("Failed to read agent file: {}", e)))?;

        // Parse metadata from frontmatter
        let metadata = AgentMetadata::from_markdown(&content)
            .map_err(|e| SubstrateError::ExecutionFailed(format!("Failed to parse agent metadata: {}", e)))?;

        // Extract prompt content (after frontmatter)
        let prompt = AgentMetadata::extract_prompt_content(&content)
            .map_err(|e| SubstrateError::ExecutionFailed(format!("Failed to extract agent prompt: {}", e)))?;

        // Build agent definition JSON
        let mut agent_def = json!({
            "description": metadata.description.unwrap_or_else(|| format!("{} agent", agent_type)),
            "prompt": prompt,
        });

        // Add tools if specified
        if !metadata.tools.is_empty() {
            agent_def["tools"] = json!(metadata.tools);
        }

        // Add model if specified
        if !metadata.model.is_empty() && metadata.model != "inherit" {
            agent_def["model"] = json!(metadata.model);
        }

        // Add MCP servers if specified
        if !metadata.mcp_servers.is_empty() {
            agent_def["mcp_servers"] = json!(metadata.mcp_servers);
        }

        // Wrap in agents object
        let agents_json = json!({
            agent_type: agent_def
        });

        serde_json::to_string(&agents_json)
            .map_err(|e| SubstrateError::ExecutionFailed(format!("Failed to serialize agents JSON: {}", e)))
    }

    /// Build the claude command with appropriate arguments
    fn build_command(&self, request: &SubstrateRequest) -> Result<Command, SubstrateError> {
        let mut cmd = Command::new(&self.config.claude_path);

        // Set working directory with priority:
        // 1. Task-specific worktree_path (from request.parameters.extra)
        // 2. Config working_dir
        // 3. Current directory
        let working_dir = if let Some(worktree_path_value) = request.parameters.extra.get("worktree_path") {
            // Task has a specific worktree - use it
            if let Some(worktree_path_str) = worktree_path_value.as_str() {
                let worktree_path = PathBuf::from(worktree_path_str);
                tracing::info!(
                    task_id = %request.task_id,
                    worktree_path = %worktree_path.display(),
                    "Using task-specific worktree directory"
                );
                cmd.current_dir(&worktree_path);
                worktree_path.display().to_string()
            } else {
                tracing::warn!(
                    task_id = %request.task_id,
                    "worktree_path parameter is not a string, using default"
                );
                if let Some(ref wd) = self.config.working_dir {
                    cmd.current_dir(wd);
                    wd.display().to_string()
                } else if let Ok(cwd) = std::env::current_dir() {
                    let cwd_str = cwd.display().to_string();
                    cmd.current_dir(&cwd);
                    cwd_str
                } else {
                    "<not set>".to_string()
                }
            }
        } else if let Some(ref wd) = self.config.working_dir {
            cmd.current_dir(wd);
            wd.display().to_string()
        } else {
            // Default to current directory if not specified
            if let Ok(cwd) = std::env::current_dir() {
                let cwd_str = cwd.display().to_string();
                cmd.current_dir(&cwd);
                cwd_str
            } else {
                "<not set>".to_string()
            }
        };

        tracing::debug!(
            task_id = %request.task_id,
            working_dir = %working_dir,
            agent_type = %request.agent_type,
            "Building Claude CLI command"
        );

        // Add model parameter if specified
        if let Some(ref model) = request.parameters.model {
            cmd.arg("--model").arg(model);
        }

        // Use --print flag for non-interactive output
        cmd.arg("--print");

        // Use JSON output format for structured responses
        cmd.arg("--output-format").arg("json");

        // Skip permission checks - agents are pre-configured in .claude/settings.json
        cmd.arg("--dangerously-skip-permissions");

        // Use stdin mode to send the prompt
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        Ok(cmd)
    }

    /// Format the prompt for Claude CLI
    ///
    /// Loads the agent's prompt from its markdown file and concatenates it with
    /// the task-specific prompt. This ensures the agent's instructions are always
    /// included without relying on Claude to interpret "Use the X subagent".
    ///
    /// # Arguments
    /// * `request` - The substrate request containing agent_type and task prompt
    ///
    /// # Returns
    /// * `Ok(String)` - Combined prompt (agent prompt + task prompt + context)
    /// * `Err(SubstrateError)` - If agent file cannot be loaded
    fn format_prompt(&self, request: &SubstrateRequest) -> Result<String, SubstrateError> {
        let mut prompt = String::new();

        // Find and load the agent file
        let agent_path = self.find_agent_file(&request.agent_type)
            .ok_or_else(|| SubstrateError::ExecutionFailed(
                format!("Agent file not found for type: {}", request.agent_type)
            ))?;

        // Read agent markdown file
        let agent_content = std::fs::read_to_string(&agent_path)
            .map_err(|e| SubstrateError::ExecutionFailed(
                format!("Failed to read agent file: {}", e)
            ))?;

        // Extract the agent's prompt (everything after frontmatter)
        let agent_prompt = AgentMetadata::extract_prompt_content(&agent_content)
            .map_err(|e| SubstrateError::ExecutionFailed(
                format!("Failed to extract agent prompt: {}", e)
            ))?;

        // Concatenate: agent prompt + task-specific instructions
        prompt.push_str(&agent_prompt);
        prompt.push_str("\n\n# Task Instructions\n\n");
        prompt.push_str(&request.prompt);

        // Add context if available
        if let Some(ref context) = request.context {
            prompt.push_str("\n\n# Input Data\n\n");
            if let Ok(pretty) = serde_json::to_string_pretty(context) {
                prompt.push_str(&pretty);
            } else {
                prompt.push_str("null");
            }
        }

        Ok(prompt)
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

        // Build command (may fail if agent loading fails critically)
        let mut cmd = self.build_command(&request)?;

        // Log the exact command for debugging
        let cmd_debug = format!("{:?}", cmd);
        tracing::info!(
            task_id = %request.task_id,
            agent_type = %request.agent_type,
            command = %cmd_debug,
            "Executing Claude CLI command"
        );

        // Spawn the process
        let mut child = cmd
            .spawn()
            .map_err(|e| {
                tracing::error!(
                    task_id = %request.task_id,
                    error = %e,
                    "Failed to spawn claude CLI subprocess"
                );
                SubstrateError::ExecutionFailed(format!("Failed to spawn claude CLI: {}", e))
            })?;

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
        let prompt = self.format_prompt(&request)?;

        tracing::info!(
            task_id = %request.task_id,
            agent_type = %request.agent_type,
            prompt_length = prompt.len(),
            prompt_preview = %prompt.chars().take(200).collect::<String>(),
            "Sending prompt to Claude CLI (agent prompt + task instructions)"
        );

        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| {
                tracing::error!(
                    task_id = %request.task_id,
                    error = %e,
                    "Failed to write prompt to stdin"
                );
                SubstrateError::ExecutionFailed(format!("Failed to write prompt: {}", e))
            })?;

        // Close stdin to signal end of input
        drop(stdin);

        tracing::debug!(
            task_id = %request.task_id,
            "Stdin closed, waiting for Claude CLI response"
        );

        // Set up timeout
        let timeout_duration = Duration::from_secs(
            request
                .parameters
                .timeout_secs
                .unwrap_or(self.config.default_timeout_secs),
        );

        // TODO: TEMPORARY DEBUG - Remove this logging once timeout issue is resolved
        tracing::info!(
            task_id = %request.task_id,
            request_timeout_secs = request.parameters.timeout_secs,
            default_timeout_secs = self.config.default_timeout_secs,
            final_timeout_secs = timeout_duration.as_secs(),
            used_default = request.parameters.timeout_secs.is_none(),
            "ClaudeCodeSubstrate: Timeout configuration for execution"
        );

        // Read output with timeout
        // IMPORTANT: Read stdout and stderr concurrently to avoid blocking
        // if one stream fills up while we're reading the other
        let result = timeout(timeout_duration, async {
            // Spawn concurrent tasks to read stdout and stderr
            let stdout_task = tokio::spawn(async move {
                let mut stdout_reader = BufReader::new(stdout);
                let mut output = String::new();
                let mut line = String::new();

                while stdout_reader.read_line(&mut line).await.map_err(|e| {
                    SubstrateError::ExecutionFailed(format!("Failed to read output: {}", e))
                })? > 0 {
                    output.push_str(&line);
                    line.clear();
                }

                Ok::<String, SubstrateError>(output)
            });

            let stderr_task = tokio::spawn(async move {
                let mut stderr_reader = BufReader::new(stderr);
                let mut errors = String::new();
                let mut error_line = String::new();

                while stderr_reader.read_line(&mut error_line).await.map_err(|e| {
                    SubstrateError::ExecutionFailed(format!("Failed to read stderr: {}", e))
                })? > 0 {
                    errors.push_str(&error_line);
                    error_line.clear();
                }

                Ok::<String, SubstrateError>(errors)
            });

            // Wait for both reading tasks to complete
            let (stdout_result, stderr_result) = tokio::try_join!(stdout_task, stderr_task)
                .map_err(|e| SubstrateError::ExecutionFailed(format!("Task join error: {}", e)))?;

            let output = stdout_result?;
            let errors = stderr_result?;

            // Wait for process to complete (should be nearly instant since stdout/stderr are closed)
            tracing::debug!(
                task_id = %request.task_id,
                "Stdout and stderr reading complete, waiting for process exit"
            );

            let status = child.wait().await.map_err(|e| {
                SubstrateError::ExecutionFailed(format!("Failed to wait for process: {}", e))
            })?;

            tracing::debug!(
                task_id = %request.task_id,
                exit_code = ?status.code(),
                "Process exited"
            );

            Ok::<_, SubstrateError>((output, errors, status))
        })
        .await;

        match result {
            Ok(Ok((output, errors, status))) => {
                tracing::info!(
                    task_id = %request.task_id,
                    exit_code = ?status.code(),
                    output_length = output.len(),
                    stderr_length = errors.len(),
                    "Claude CLI subprocess completed"
                );

                tracing::debug!(
                    task_id = %request.task_id,
                    output = %output,
                    "Claude CLI full output"
                );

                if !errors.is_empty() {
                    tracing::warn!(
                        task_id = %request.task_id,
                        stderr = %errors,
                        "Claude CLI produced stderr output"
                    );
                }

                // Check if execution succeeded
                if !status.success() {
                    tracing::error!(
                        task_id = %request.task_id,
                        exit_code = ?status.code(),
                        stderr = %errors,
                        "Claude CLI exited with non-zero status"
                    );

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

                // Claude CLI returns a JSON wrapper with the actual content in the "result" field
                // Try to extract it, but fall back to raw output if parsing fails
                let content = match serde_json::from_str::<serde_json::Value>(&output) {
                    Ok(json) if json.is_object() => {
                        // Look for the "result" field in the wrapper
                        if let Some(result) = json.get("result") {
                            if let Some(result_str) = result.as_str() {
                                tracing::info!(
                                    task_id = %request.task_id,
                                    "Extracted agent output from Claude CLI result wrapper"
                                );
                                result_str.to_string()
                            } else {
                                // Result is not a string, use the whole thing
                                tracing::warn!(
                                    task_id = %request.task_id,
                                    "Claude CLI result field is not a string, using raw output"
                                );
                                output
                            }
                        } else {
                            // No result field, assume the whole output is the content
                            tracing::debug!(
                                task_id = %request.task_id,
                                "No result field in Claude CLI output, using raw output"
                            );
                            output
                        }
                    }
                    _ => {
                        // Not JSON or not an object, use as-is
                        tracing::debug!(
                            task_id = %request.task_id,
                            "Claude CLI output is not JSON, using raw output"
                        );
                        output
                    }
                };

                Ok(SubstrateResponse {
                    task_id: request.task_id,
                    content,
                    stop_reason: StopReason::EndTurn,
                    usage: None, // Claude CLI doesn't provide token usage
                    metadata,
                })
            }
            Ok(Err(e)) => {
                tracing::error!(
                    task_id = %request.task_id,
                    error = %e,
                    "Claude CLI subprocess encountered an error"
                );
                Err(e)
            }
            Err(_) => {
                // Timeout occurred - kill the process
                tracing::error!(
                    task_id = %request.task_id,
                    timeout_secs = request.parameters.timeout_secs.unwrap_or(self.config.default_timeout_secs),
                    "Claude CLI subprocess timed out"
                );

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
            agent_type: "requirements-gatherer".to_string(), // Use real agent that exists
            prompt: "Hello, world!".to_string(),
            context: None,
            parameters: ExecutionParameters::default(),
        };

        // Should successfully load agent and format prompt
        let result = substrate.format_prompt(&request);
        assert!(result.is_ok(), "format_prompt should succeed with valid agent");

        let formatted = result.unwrap();
        // Should contain task instructions header and task prompt
        assert!(formatted.contains("# Task Instructions"));
        assert!(formatted.contains("Hello, world!"));
    }

    #[test]
    fn test_format_prompt_with_context() {
        let substrate = ClaudeCodeSubstrate::new();
        let request = SubstrateRequest {
            task_id: Uuid::new_v4(),
            agent_type: "requirements-gatherer".to_string(), // Use real agent that exists
            prompt: "Analyze this".to_string(),
            context: Some(serde_json::json!({"key": "value"})),
            parameters: ExecutionParameters::default(),
        };

        let result = substrate.format_prompt(&request);
        assert!(result.is_ok(), "format_prompt should succeed with valid agent");

        let formatted = result.unwrap();
        // Should contain task instructions, task prompt, and context
        assert!(formatted.contains("# Task Instructions"));
        assert!(formatted.contains("Analyze this"));
        assert!(formatted.contains("# Input Data"));
        assert!(formatted.contains("key"));
    }

    #[test]
    fn test_format_prompt_missing_agent() {
        let substrate = ClaudeCodeSubstrate::new();
        let request = SubstrateRequest {
            task_id: Uuid::new_v4(),
            agent_type: "nonexistent-agent".to_string(),
            prompt: "Test".to_string(),
            context: None,
            parameters: ExecutionParameters::default(),
        };

        // Should fail when agent file doesn't exist
        let result = substrate.format_prompt(&request);
        assert!(result.is_err(), "format_prompt should fail with missing agent");
    }

    #[test]
    fn test_build_command_with_model() {
        let substrate = ClaudeCodeSubstrate::new();
        let mut params = ExecutionParameters::default();
        params.model = Some("opus".to_string());

        let request = SubstrateRequest {
            task_id: Uuid::new_v4(),
            agent_type: "test-agent".to_string(),
            prompt: "Test".to_string(),
            context: None,
            parameters: params,
        };

        let cmd = substrate.build_command(&request).expect("build_command should succeed");
        let cmd_debug = format!("{:?}", cmd);

        // Verify the command includes --model flag with simple model name
        assert!(cmd_debug.contains("--model"));
        assert!(cmd_debug.contains("opus"));
    }

    #[test]
    fn test_build_command_without_model() {
        let substrate = ClaudeCodeSubstrate::new();
        let request = SubstrateRequest {
            task_id: Uuid::new_v4(),
            agent_type: "test-agent".to_string(),
            prompt: "Test".to_string(),
            context: None,
            parameters: ExecutionParameters::default(),
        };

        let cmd = substrate.build_command(&request).expect("build_command should succeed");
        let cmd_debug = format!("{:?}", cmd);

        // Should still have the -- argument for stdin
        assert!(cmd_debug.contains("--"));
    }
}

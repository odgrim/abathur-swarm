//! Claude Code CLI substrate implementation.
//!
//! Spawns Claude Code CLI processes to execute agent tasks.

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    SessionStatus, SubstrateOutput, SubstrateRequest, SubstrateSession,
};
use crate::domain::ports::Substrate;

/// Claude Code CLI substrate configuration.
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// Path to claude CLI binary
    pub binary_path: String,
    /// Default model to use
    pub default_model: String,
    /// Default max turns
    pub default_max_turns: u32,
    /// Session storage directory
    pub session_dir: PathBuf,
    /// Whether to use --print mode
    pub print_mode: bool,
    /// Output format for print mode (text, json, stream-json)
    pub output_format: String,
    /// Additional CLI flags
    pub extra_flags: Vec<String>,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            binary_path: "claude".to_string(),
            default_model: "sonnet".to_string(),
            default_max_turns: 25,
            session_dir: PathBuf::from(".abathur/sessions"),
            print_mode: true,
            output_format: "stream-json".to_string(),
            extra_flags: vec![],
        }
    }
}

/// Claude Code CLI substrate.
pub struct ClaudeCodeSubstrate {
    config: ClaudeCodeConfig,
    sessions: Arc<RwLock<HashMap<Uuid, SubstrateSession>>>,
    running_processes: Arc<RwLock<HashMap<Uuid, u32>>>,
}

impl ClaudeCodeSubstrate {
    pub fn new(config: ClaudeCodeConfig) -> Self {
        Self {
            config,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            running_processes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Build CLI arguments for a request.
    fn build_args(&self, request: &SubstrateRequest) -> Vec<String> {
        let mut args = vec![];

        // Print mode for non-interactive execution
        if self.config.print_mode {
            args.push("--print".to_string());
            // Output format for structured output
            args.push("--output-format".to_string());
            args.push(self.config.output_format.clone());
        }

        // Max turns
        let max_turns = request.config.max_turns;
        args.push("--max-turns".to_string());
        args.push(max_turns.to_string());

        // Model selection
        if let Some(ref model) = request.config.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        // System prompt (only add if non-empty)
        if !request.system_prompt.is_empty() {
            args.push("--system-prompt".to_string());
            args.push(request.system_prompt.clone());
        }

        // MCP servers
        for server in &request.config.mcp_servers {
            args.push("--mcp".to_string());
            args.push(server.clone());
        }

        // Allowed tools - use full set by default
        if request.config.allow_tools {
            args.push("--allowedTools".to_string());
            args.push("Edit,Write,Bash,Glob,Grep,Read,TodoWrite,WebFetch,WebSearch,Task,MultiEdit".to_string());
        }

        // Allowed files patterns
        for pattern in &request.config.allowed_files {
            args.push("--allowedFiles".to_string());
            args.push(pattern.clone());
        }

        // Denied files patterns
        for pattern in &request.config.denied_files {
            args.push("--deniedFiles".to_string());
            args.push(pattern.clone());
        }

        // Extra flags from config
        args.extend(self.config.extra_flags.clone());

        // The prompt itself
        args.push("-p".to_string());
        args.push(request.user_prompt.clone());

        args
    }

    /// Parse streaming JSON output from claude CLI (stream-json format).
    /// Each line is a JSON object with different event types.
    fn parse_stream_json(line: &str) -> Option<SubstrateOutput> {
        let json: serde_json::Value = serde_json::from_str(line).ok()?;

        // Check for different event types based on Claude Code stream-json format
        let event_type = json.get("type").and_then(|t| t.as_str())?;

        match event_type {
            // Assistant message content
            "assistant" | "content_block_delta" | "text" => {
                let content = json.get("content")
                    .or_else(|| json.get("text"))
                    .or_else(|| json.get("delta").and_then(|d| d.get("text")))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");

                if !content.is_empty() {
                    Some(SubstrateOutput::AssistantText {
                        content: content.to_string(),
                    })
                } else {
                    None
                }
            }

            // Tool use started
            "tool_use" | "tool_use_block" => {
                Some(SubstrateOutput::ToolStart {
                    name: json.get("name").and_then(|n| n.as_str()).unwrap_or("unknown").to_string(),
                    id: json.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
                })
            }

            // Tool result
            "tool_result" => {
                Some(SubstrateOutput::ToolResult {
                    id: json.get("tool_use_id")
                        .or_else(|| json.get("id"))
                        .and_then(|i| i.as_str())
                        .unwrap_or("")
                        .to_string(),
                    result: json.get("content")
                        .or_else(|| json.get("result"))
                        .and_then(|r| {
                            if r.is_string() {
                                r.as_str().map(|s| s.to_string())
                            } else {
                                Some(r.to_string())
                            }
                        })
                        .unwrap_or_default(),
                    is_error: json.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false),
                })
            }

            // Usage/token information
            "usage" | "message_delta" => {
                let usage = json.get("usage").unwrap_or(&json);
                let input = usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                let output = usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);

                if input > 0 || output > 0 {
                    Some(SubstrateOutput::TurnComplete {
                        turn_number: 0,
                        input_tokens: input,
                        output_tokens: output,
                    })
                } else {
                    None
                }
            }

            // Result/completion event
            "result" | "message_stop" => {
                let result = json.get("result")
                    .or_else(|| json.get("content"))
                    .and_then(|r| {
                        if r.is_string() {
                            r.as_str().map(|s| s.to_string())
                        } else {
                            Some(r.to_string())
                        }
                    })
                    .unwrap_or_else(|| "Completed".to_string());

                // Extract final usage if present
                if let Some(usage) = json.get("usage") {
                    let input = usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                    let output = usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);

                    // Return usage first, then session complete will follow
                    if input > 0 || output > 0 {
                        return Some(SubstrateOutput::TurnComplete {
                            turn_number: 0,
                            input_tokens: input,
                            output_tokens: output,
                        });
                    }
                }

                Some(SubstrateOutput::SessionComplete { result })
            }

            // Error events
            "error" => {
                let message = json.get("error")
                    .and_then(|e| e.get("message"))
                    .or_else(|| json.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error")
                    .to_string();

                Some(SubstrateOutput::Error { message })
            }

            // Status updates
            "system" | "status" | "ping" => {
                json.get("message")
                    .and_then(|m| m.as_str())
                    .map(|msg| SubstrateOutput::Status {
                        message: msg.to_string(),
                    })
            }

            // Ignore other event types
            _ => None,
        }
    }

    /// Parse a line of output (handles both JSON and plain text).
    fn parse_output_line(line: &str) -> Option<SubstrateOutput> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        // Try JSON first
        if trimmed.starts_with('{') {
            if let Some(output) = Self::parse_stream_json(trimmed) {
                return Some(output);
            }
        }

        // Fall back to plain text
        Some(SubstrateOutput::AssistantText {
            content: line.to_string(),
        })
    }

    /// Parse the final result to extract token usage from the output.
    fn extract_final_usage(output: &str) -> (u64, u64, u64, u64) {
        // Try to find usage information in the output
        // Claude Code outputs usage stats at the end
        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;
        let mut cache_read = 0u64;
        let mut cache_write = 0u64;

        for line in output.lines() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(usage) = json.get("usage") {
                    input_tokens += usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                    output_tokens += usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                    cache_read += usage.get("cache_read_input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                    cache_write += usage.get("cache_creation_input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                }
            }
        }

        (input_tokens, output_tokens, cache_read, cache_write)
    }
}

#[async_trait]
impl Substrate for ClaudeCodeSubstrate {
    fn name(&self) -> &'static str {
        "claude_code"
    }

    async fn is_available(&self) -> DomainResult<bool> {
        let output = Command::new(&self.config.binary_path)
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(out) => Ok(out.status.success()),
            Err(_) => Ok(false),
        }
    }

    async fn execute(&self, request: SubstrateRequest) -> DomainResult<SubstrateSession> {
        let args = self.build_args(&request);
        let working_dir = request.config.working_dir
            .clone()
            .unwrap_or_else(|| ".".to_string());

        let mut cmd = Command::new(&self.config.binary_path);
        cmd.args(&args)
            .current_dir(&working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Set environment variables
        for (key, value) in &request.config.env_vars {
            cmd.env(key, value);
        }

        // Set task-specific environment
        cmd.env("ABATHUR_TASK_ID", request.task_id.to_string());

        let mut child = cmd.spawn()
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to spawn claude: {}", e)))?;

        let pid = child.id();

        // Create session
        let mut session = SubstrateSession::new(request.task_id, &request.agent_template, request.config.clone());
        session.start(pid);

        // Store session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session.id, session.clone());
        }

        // Track running process
        if let Some(pid) = pid {
            let mut processes = self.running_processes.write().await;
            processes.insert(session.id, pid);
        }

        // Read stdout
        let stdout = child.stdout.take()
            .ok_or_else(|| DomainError::ValidationFailed("Failed to capture stdout".to_string()))?;
        let stderr = child.stderr.take()
            .ok_or_else(|| DomainError::ValidationFailed("Failed to capture stderr".to_string()))?;

        // Collect output
        let mut output_text = String::new();
        let mut error_text = String::new();
        let mut total_input_tokens = 0u64;
        let mut total_output_tokens = 0u64;
        let mut turns = 0u32;

        // Read stdout line by line
        let stdout_reader = BufReader::new(stdout);
        let mut stdout_lines = stdout_reader.lines();

        while let Ok(Some(line)) = stdout_lines.next_line().await {
            output_text.push_str(&line);
            output_text.push('\n');

            if let Some(parsed) = Self::parse_output_line(&line) {
                match parsed {
                    SubstrateOutput::TurnComplete { input_tokens, output_tokens, .. } => {
                        total_input_tokens += input_tokens;
                        total_output_tokens += output_tokens;
                        turns += 1;
                    }
                    SubstrateOutput::Error { message } => {
                        error_text.push_str(&message);
                        error_text.push('\n');
                    }
                    _ => {}
                }
            }
        }

        // Read stderr
        let stderr_reader = BufReader::new(stderr);
        let mut stderr_lines = stderr_reader.lines();
        while let Ok(Some(line)) = stderr_lines.next_line().await {
            error_text.push_str(&line);
            error_text.push('\n');
        }

        // Wait for process to complete
        let exit_status = child.wait().await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to wait for process: {}", e)))?;

        // Remove from running processes
        {
            let mut processes = self.running_processes.write().await;
            processes.remove(&session.id);
        }

        // If we didn't get usage from streaming, try to extract from final output
        if total_input_tokens == 0 && total_output_tokens == 0 {
            let (input, output, cache_read, cache_write) = Self::extract_final_usage(&output_text);
            total_input_tokens = input;
            total_output_tokens = output;
            session.cache_read_tokens = cache_read;
            session.cache_write_tokens = cache_write;
        }

        // Update session with token counts
        session.input_tokens = total_input_tokens;
        session.output_tokens = total_output_tokens;
        session.turns_completed = turns.max(1); // At least 1 turn

        // Determine success based on exit code
        if exit_status.success() {
            session.complete(output_text.trim());
        } else {
            let error_msg = if !error_text.trim().is_empty() {
                error_text.trim().to_string()
            } else {
                format!("Process exited with code: {:?}", exit_status.code())
            };
            session.fail(&error_msg);
        }

        // Update session store
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session.id, session.clone());
        }

        Ok(session)
    }

    async fn execute_streaming(
        &self,
        request: SubstrateRequest,
    ) -> DomainResult<(mpsc::Receiver<SubstrateOutput>, SubstrateSession)> {
        let args = self.build_args(&request);
        let working_dir = request.config.working_dir
            .clone()
            .unwrap_or_else(|| ".".to_string());

        let mut cmd = Command::new(&self.config.binary_path);
        cmd.args(&args)
            .current_dir(&working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Set environment variables
        for (key, value) in &request.config.env_vars {
            cmd.env(key, value);
        }

        // Set task-specific environment
        cmd.env("ABATHUR_TASK_ID", request.task_id.to_string());

        let mut child = cmd.spawn()
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to spawn claude: {}", e)))?;

        let pid = child.id();

        // Create session
        let mut session = SubstrateSession::new(request.task_id, &request.agent_template, request.config);
        session.start(pid);

        // Store session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session.id, session.clone());
        }

        // Track running process
        if let Some(pid) = pid {
            let mut processes = self.running_processes.write().await;
            processes.insert(session.id, pid);
        }

        // Create output channel
        let (tx, rx) = mpsc::channel(100);

        // Get handles to stdout and stderr
        let stdout = child.stdout.take()
            .ok_or_else(|| DomainError::ValidationFailed("Failed to capture stdout".to_string()))?;
        let stderr = child.stderr.take()
            .ok_or_else(|| DomainError::ValidationFailed("Failed to capture stderr".to_string()))?;

        let session_id = session.id;
        let sessions_clone = self.sessions.clone();
        let processes_clone = self.running_processes.clone();

        // Spawn task to read and stream output
        tokio::spawn(async move {
            let stdout_reader = BufReader::new(stdout);
            let stderr_reader = BufReader::new(stderr);
            let mut stdout_lines = stdout_reader.lines();
            let mut stderr_lines = stderr_reader.lines();

            let mut all_output = String::new();
            let mut total_input = 0u64;
            let mut total_output = 0u64;

            // Read stdout
            while let Ok(Some(line)) = stdout_lines.next_line().await {
                all_output.push_str(&line);
                all_output.push('\n');

                if let Some(output) = Self::parse_output_line(&line) {
                    match &output {
                        SubstrateOutput::TurnComplete { input_tokens, output_tokens, .. } => {
                            total_input += input_tokens;
                            total_output += output_tokens;
                        }
                        _ => {}
                    }
                    if tx.send(output).await.is_err() {
                        break;
                    }
                }
            }

            // Read any stderr
            let mut error_output = String::new();
            while let Ok(Some(line)) = stderr_lines.next_line().await {
                error_output.push_str(&line);
                error_output.push('\n');
            }

            // Wait for process and check exit status
            let exit_result = child.wait().await;

            // Remove from running processes
            {
                let mut processes = processes_clone.write().await;
                processes.remove(&session_id);
            }

            // Update session status based on exit
            {
                let mut sessions = sessions_clone.write().await;
                if let Some(session) = sessions.get_mut(&session_id) {
                    session.input_tokens = total_input;
                    session.output_tokens = total_output;

                    match exit_result {
                        Ok(status) if status.success() => {
                            if session.status == SessionStatus::Active {
                                session.complete(all_output.trim());
                            }
                            let _ = tx.send(SubstrateOutput::SessionComplete {
                                result: "Completed successfully".to_string(),
                            }).await;
                        }
                        Ok(status) => {
                            let error = if !error_output.trim().is_empty() {
                                error_output.trim().to_string()
                            } else {
                                format!("Exit code: {:?}", status.code())
                            };
                            session.fail(&error);
                            let _ = tx.send(SubstrateOutput::Error { message: error }).await;
                        }
                        Err(e) => {
                            session.fail(&e.to_string());
                            let _ = tx.send(SubstrateOutput::Error {
                                message: e.to_string(),
                            }).await;
                        }
                    }
                }
            }
        });

        Ok((rx, session))
    }

    async fn resume(
        &self,
        session_id: Uuid,
        additional_prompt: Option<String>,
    ) -> DomainResult<SubstrateSession> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&session_id)
            .ok_or_else(|| DomainError::ValidationFailed(format!("Session {} not found", session_id)))?;

        if !session.status.is_terminal() {
            return Err(DomainError::ValidationFailed(
                "Cannot resume active session".to_string()
            ));
        }

        // Create a new request based on the original session
        let mut config = session.config.clone();
        config.max_turns = config.max_turns.saturating_sub(session.turns_completed);

        let request = SubstrateRequest {
            task_id: session.task_id,
            agent_template: session.agent_template.clone(),
            system_prompt: String::new(), // Claude Code maintains context
            user_prompt: additional_prompt.unwrap_or_else(|| "Continue with the task.".to_string()),
            config,
            resume_session: Some(session_id),
        };

        drop(sessions);
        self.execute(request).await
    }

    async fn terminate(&self, session_id: Uuid) -> DomainResult<()> {
        let processes = self.running_processes.read().await;
        if let Some(&pid) = processes.get(&session_id) {
            // Kill the process
            #[cfg(unix)]
            {
                let _ = std::process::Command::new("kill")
                    .arg("-9")
                    .arg(pid.to_string())
                    .output();
            }

            #[cfg(not(unix))]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .output();
            }
        }
        drop(processes);

        // Update session status
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            session.terminate();
        }

        // Remove from running processes
        let mut processes = self.running_processes.write().await;
        processes.remove(&session_id);

        Ok(())
    }

    async fn get_session(&self, session_id: Uuid) -> DomainResult<Option<SubstrateSession>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(&session_id).cloned())
    }

    async fn is_running(&self, session_id: Uuid) -> DomainResult<bool> {
        let processes = self.running_processes.read().await;
        Ok(processes.contains_key(&session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::SubstrateConfig;

    #[test]
    fn test_build_args() {
        let config = ClaudeCodeConfig::default();
        let substrate = ClaudeCodeSubstrate::new(config);

        let request = SubstrateRequest::new(
            Uuid::new_v4(),
            "test-agent",
            "You are a helpful assistant",
            "Hello world",
        ).with_config(SubstrateConfig::default().with_max_turns(10));

        let args = substrate.build_args(&request);

        assert!(args.contains(&"--print".to_string()));
        assert!(args.contains(&"--max-turns".to_string()));
        assert!(args.contains(&"10".to_string()));
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"--output-format".to_string()));
    }

    #[test]
    fn test_parse_stream_json_assistant() {
        let line = r#"{"type":"assistant","content":"Hello!"}"#;
        let output = ClaudeCodeSubstrate::parse_stream_json(line);
        assert!(matches!(output, Some(SubstrateOutput::AssistantText { content }) if content == "Hello!"));
    }

    #[test]
    fn test_parse_stream_json_tool_use() {
        let line = r#"{"type":"tool_use","name":"Read","id":"tool_123"}"#;
        let output = ClaudeCodeSubstrate::parse_stream_json(line);
        assert!(matches!(output, Some(SubstrateOutput::ToolStart { name, id }) if name == "Read" && id == "tool_123"));
    }

    #[test]
    fn test_parse_stream_json_usage() {
        let line = r#"{"type":"usage","usage":{"input_tokens":100,"output_tokens":50}}"#;
        let output = ClaudeCodeSubstrate::parse_stream_json(line);
        assert!(matches!(
            output,
            Some(SubstrateOutput::TurnComplete { input_tokens: 100, output_tokens: 50, .. })
        ));
    }

    #[test]
    fn test_parse_stream_json_error() {
        let line = r#"{"type":"error","message":"Something went wrong"}"#;
        let output = ClaudeCodeSubstrate::parse_stream_json(line);
        assert!(matches!(output, Some(SubstrateOutput::Error { message }) if message == "Something went wrong"));
    }

    #[test]
    fn test_parse_output_plain_text() {
        let output = ClaudeCodeSubstrate::parse_output_line("Hello world");
        assert!(matches!(output, Some(SubstrateOutput::AssistantText { .. })));
    }

    #[test]
    fn test_parse_output_empty() {
        let output = ClaudeCodeSubstrate::parse_output_line("");
        assert!(output.is_none());

        let output = ClaudeCodeSubstrate::parse_output_line("   ");
        assert!(output.is_none());
    }

    #[test]
    fn test_config_defaults() {
        let config = ClaudeCodeConfig::default();
        assert_eq!(config.binary_path, "claude");
        assert_eq!(config.default_model, "sonnet");
        assert_eq!(config.default_max_turns, 25);
        assert!(config.print_mode);
        assert_eq!(config.output_format, "stream-json");
    }
}

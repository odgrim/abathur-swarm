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

        // System prompt
        args.push("--system-prompt".to_string());
        args.push(request.system_prompt.clone());

        // MCP servers
        for server in &request.config.mcp_servers {
            args.push("--mcp".to_string());
            args.push(server.clone());
        }

        // Allowed tools
        if request.config.allow_tools {
            args.push("--allowedTools".to_string());
            args.push("Edit,Write,Bash,Glob,Grep,Read,TodoWrite,WebFetch,WebSearch".to_string());
        }

        // Extra flags from config
        args.extend(self.config.extra_flags.clone());

        // The prompt itself
        args.push("-p".to_string());
        args.push(request.user_prompt.clone());

        args
    }

    /// Parse streaming output from claude CLI.
    fn parse_output_line(line: &str) -> Option<SubstrateOutput> {
        // Try to parse as JSON first (structured output)
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(event_type) = json.get("type").and_then(|t| t.as_str()) {
                return match event_type {
                    "assistant" => json.get("content")
                        .and_then(|c| c.as_str())
                        .map(|content| SubstrateOutput::AssistantText {
                            content: content.to_string(),
                        }),
                    "tool_use" => Some(SubstrateOutput::ToolStart {
                        name: json.get("name").and_then(|n| n.as_str()).unwrap_or("unknown").to_string(),
                        id: json.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
                    }),
                    "tool_result" => Some(SubstrateOutput::ToolResult {
                        id: json.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
                        result: json.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string(),
                        is_error: json.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false),
                    }),
                    "usage" => {
                        let input = json.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                        let output = json.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                        Some(SubstrateOutput::TurnComplete {
                            turn_number: 0,
                            input_tokens: input,
                            output_tokens: output,
                        })
                    }
                    "error" => Some(SubstrateOutput::Error {
                        message: json.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error").to_string(),
                    }),
                    _ => None,
                };
            }
        }

        // Plain text output
        if !line.trim().is_empty() {
            Some(SubstrateOutput::AssistantText {
                content: line.to_string(),
            })
        } else {
            None
        }
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
        let (mut rx, session) = self.execute_streaming(request).await?;

        // Drain the channel and collect final state
        let mut final_session = session;
        let mut output_text = String::new();

        while let Some(output) = rx.recv().await {
            match output {
                SubstrateOutput::AssistantText { content } => {
                    output_text.push_str(&content);
                    output_text.push('\n');
                }
                SubstrateOutput::TurnComplete { input_tokens, output_tokens, .. } => {
                    final_session.record_turn(input_tokens, output_tokens);
                }
                SubstrateOutput::Error { message } => {
                    final_session.fail(&message);
                    return Ok(final_session);
                }
                SubstrateOutput::SessionComplete { result } => {
                    final_session.complete(&result);
                    return Ok(final_session);
                }
                _ => {}
            }
        }

        // If we got here, session completed normally
        if final_session.status == SessionStatus::Active {
            final_session.complete(output_text.trim());
        }

        // Update session store
        let mut sessions = self.sessions.write().await;
        sessions.insert(final_session.id, final_session.clone());

        Ok(final_session)
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

        // Spawn task to read output
        let stdout = child.stdout.take()
            .ok_or_else(|| DomainError::ValidationFailed("Failed to capture stdout".to_string()))?;

        let session_id = session.id;
        let sessions_clone = self.sessions.clone();
        let processes_clone = self.running_processes.clone();

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(output) = Self::parse_output_line(&line) {
                    if tx.send(output).await.is_err() {
                        break;
                    }
                }
            }

            // Wait for process to finish
            let _ = child.wait().await;

            // Remove from running processes
            let mut processes = processes_clone.write().await;
            processes.remove(&session_id);

            // Update session status
            let mut sessions = sessions_clone.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                if session.status == SessionStatus::Active {
                    session.status = SessionStatus::Completed;
                    session.ended_at = Some(chrono::Utc::now());
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
                use std::os::unix::process::CommandExt;
                let _ = std::process::Command::new("kill")
                    .arg("-9")
                    .arg(pid.to_string())
                    .exec();
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
    }

    #[test]
    fn test_parse_output() {
        // Plain text
        let output = ClaudeCodeSubstrate::parse_output_line("Hello world");
        assert!(matches!(output, Some(SubstrateOutput::AssistantText { .. })));

        // Empty line
        let output = ClaudeCodeSubstrate::parse_output_line("");
        assert!(output.is_none());

        // JSON error
        let output = ClaudeCodeSubstrate::parse_output_line(r#"{"type":"error","message":"test"}"#);
        assert!(matches!(output, Some(SubstrateOutput::Error { .. })));
    }
}

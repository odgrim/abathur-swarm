//! Swarm service for CLI interaction
//!
//! Manages the lifecycle of swarm orchestrator state.
//! Since each CLI invocation is a separate process, state is persisted to disk.

use crate::application::{SwarmState, SwarmStats};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Persisted swarm state - only tracks process information
/// Task statistics are derived from the database, not stored here
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SwarmStateFile {
    state: String,
    max_agents: usize,
    pid: Option<u32>,
    memory_server_pid: Option<u32>,
    tasks_server_pid: Option<u32>,
}

impl Default for SwarmStateFile {
    fn default() -> Self {
        Self {
            state: "Stopped".to_string(),
            max_agents: 0,
            pid: None,
            memory_server_pid: None,
            tasks_server_pid: None,
        }
    }
}

/// Swarm service for CLI commands
pub struct SwarmService;

impl SwarmService {
    /// Create a new swarm service
    pub const fn new() -> Self {
        Self
    }

    /// Get the path to the swarm state file (project-local)
    fn state_file_path() -> Result<PathBuf> {
        let current_dir = std::env::current_dir()
            .context("Failed to get current directory")?;
        let config_dir = current_dir.join(".abathur");
        fs::create_dir_all(&config_dir).context("Failed to create config directory")?;
        Ok(config_dir.join("swarm_state.json"))
    }

    /// Check if a process with the given PID is running
    fn is_process_alive(pid: u32) -> bool {
        #[cfg(unix)]
        {
            use std::process::{Command, Stdio};
            Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        }

        #[cfg(windows)]
        {
            use std::process::Command;
            Command::new("tasklist")
                .args(&["/FI", &format!("PID eq {}", pid)])
                .output()
                .map(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .contains(&pid.to_string())
                })
                .unwrap_or(false)
        }

        #[cfg(not(any(unix, windows)))]
        {
            // Unsupported platform, assume not running
            false
        }
    }

    /// Read swarm state from file
    fn read_state() -> Result<SwarmStateFile> {
        let path = Self::state_file_path()?;
        if !path.exists() {
            return Ok(SwarmStateFile::default());
        }

        let contents = fs::read_to_string(&path)
            .context("Failed to read swarm state file")?;
        serde_json::from_str(&contents)
            .context("Failed to parse swarm state file")
    }

    /// Write swarm state to file
    fn write_state(state: &SwarmStateFile) -> Result<()> {
        let path = Self::state_file_path()?;
        let contents = serde_json::to_string_pretty(state)
            .context("Failed to serialize swarm state")?;
        fs::write(&path, contents)
            .context("Failed to write swarm state file")
    }

    /// Initialize and start the swarm orchestrator
    pub async fn start(&self, max_agents: usize) -> Result<()> {
        let mut state = Self::read_state()?;

        // Check if already running (both state and PID check)
        if matches!(state.state.as_str(), "Running" | "Starting") {
            if let Some(pid) = state.pid {
                if Self::is_process_alive(pid) {
                    return Ok(()); // Already running
                }
                // PID exists but process is dead, continue to restart
            }
        }

        // Get the current executable path
        let exe_path = std::env::current_exe()
            .context("Failed to get current executable path")?;

        // Get current working directory for the child process
        let cwd = std::env::current_dir()
            .context("Failed to get current directory")?;

        // Create log file for daemon stderr/stdout
        let log_path = cwd.join(".abathur/swarm_daemon.log");
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .context("Failed to create daemon log file")?;

        // Write log header
        use std::io::Write;
        let mut header_file = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)?;
        writeln!(header_file, "\n=== Daemon starting at {} ===",
                 chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))?;

        // Spawn background process with hidden --daemon flag
        // This flag will be handled in main.rs to run the orchestrator loop
        #[cfg(unix)]
        let child = Command::new(&exe_path)
            .arg("swarm")
            .arg("start")
            .arg("--daemon")
            .arg("--max-agents")
            .arg(max_agents.to_string())
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file.try_clone()?))
            .stderr(Stdio::from(log_file))
            .spawn()
            .context("Failed to spawn background swarm process")?;

        #[cfg(windows)]
        let child = Command::new(&exe_path)
            .arg("swarm")
            .arg("start")
            .arg("--daemon")
            .arg("--max-agents")
            .arg(max_agents.to_string())
            .current_dir(&cwd)
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file.try_clone()?))
            .stderr(Stdio::from(log_file))
            .spawn()
            .context("Failed to spawn background swarm process")?;

        let child_pid = child.id();

        // Update state with child PID
        state.state = "Running".to_string();
        state.max_agents = max_agents;
        state.pid = Some(child_pid);
        Self::write_state(&state)?;

        Ok(())
    }

    /// Stop the swarm orchestrator
    pub async fn stop(&self) -> Result<()> {
        let mut state = Self::read_state()?;

        // Check if there's actually a PID to stop
        let Some(pid) = state.pid else {
            anyhow::bail!("No swarm orchestrator is running (no PID found)");
        };

        // Check if the process is actually alive
        if !Self::is_process_alive(pid) {
            // PID exists but process is dead - clean up state and stop orphaned MCP servers
            Self::stop_mcp_servers(&state).await;
            state.state = "Stopped".to_string();
            state.pid = None;
            state.memory_server_pid = None;
            state.tasks_server_pid = None;
            Self::write_state(&state)?;
            anyhow::bail!("No swarm orchestrator is running (PID {} not found)", pid);
        }

        // Stop MCP servers first (before stopping the daemon)
        Self::stop_mcp_servers(&state).await;

        // Process is alive, try to kill it
        #[cfg(unix)]
        {
            use std::process::Command;
            // Send SIGTERM for graceful shutdown
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status();

            // Wait briefly for graceful shutdown
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            // Force kill if still alive
            if Self::is_process_alive(pid) {
                let _ = Command::new("kill")
                    .arg("-KILL")
                    .arg(pid.to_string())
                    .status();
            }
        }

        #[cfg(windows)]
        {
            use std::process::Command;
            let _ = Command::new("taskkill")
                .args(&["/PID", &pid.to_string(), "/F"])
                .status();
        }

        state.state = "Stopped".to_string();
        state.pid = None;
        state.memory_server_pid = None;
        state.tasks_server_pid = None;
        Self::write_state(&state)?;

        Ok(())
    }

    /// Stop MCP servers if they are running
    async fn stop_mcp_servers(state: &SwarmStateFile) {
        // Stop memory server
        if let Some(memory_pid) = state.memory_server_pid {
            if Self::is_process_alive(memory_pid) {
                #[cfg(unix)]
                {
                    use std::process::Command;
                    let _ = Command::new("kill")
                        .arg("-TERM")
                        .arg(memory_pid.to_string())
                        .status();

                    // Wait briefly for graceful shutdown
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                    // Force kill if still alive
                    if Self::is_process_alive(memory_pid) {
                        let _ = Command::new("kill")
                            .arg("-KILL")
                            .arg(memory_pid.to_string())
                            .status();
                    }
                }

                #[cfg(windows)]
                {
                    use std::process::Command;
                    let _ = Command::new("taskkill")
                        .args(&["/PID", &memory_pid.to_string(), "/F"])
                        .status();
                }
            }
        }

        // Stop tasks server
        if let Some(tasks_pid) = state.tasks_server_pid {
            if Self::is_process_alive(tasks_pid) {
                #[cfg(unix)]
                {
                    use std::process::Command;
                    let _ = Command::new("kill")
                        .arg("-TERM")
                        .arg(tasks_pid.to_string())
                        .status();

                    // Wait briefly for graceful shutdown
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                    // Force kill if still alive
                    if Self::is_process_alive(tasks_pid) {
                        let _ = Command::new("kill")
                            .arg("-KILL")
                            .arg(tasks_pid.to_string())
                            .status();
                    }
                }

                #[cfg(windows)]
                {
                    use std::process::Command;
                    let _ = Command::new("taskkill")
                        .args(&["/PID", &tasks_pid.to_string(), "/F"])
                        .status();
                }
            }
        }
    }

    /// Get swarm statistics
    ///
    /// Combines process state from state file with task statistics from database
    pub async fn get_stats(
        &self,
        task_service: &crate::cli::service::TaskQueueServiceAdapter,
    ) -> Result<SwarmStats> {
        let mut state = Self::read_state()?;

        // Check if the recorded PID is actually alive
        let process_alive = state.pid.map_or(false, Self::is_process_alive);

        // Correct the state if necessary
        let swarm_state = match state.state.as_str() {
            "Starting" | "Running" => {
                if process_alive {
                    SwarmState::Running
                } else {
                    // Process died, update state file
                    state.state = "Stopped".to_string();
                    state.pid = None;
                    let _ = Self::write_state(&state); // Best effort update
                    SwarmState::Stopped
                }
            }
            "Stopping" => {
                if process_alive {
                    SwarmState::Stopping
                } else {
                    // Process finished stopping
                    state.state = "Stopped".to_string();
                    state.pid = None;
                    let _ = Self::write_state(&state);
                    SwarmState::Stopped
                }
            }
            _ => SwarmState::Stopped,
        };

        // Get task statistics from database (source of truth)
        let queue_stats = task_service.get_queue_stats().await?;

        // Calculate agent statistics
        let active_agents = queue_stats.running;
        let idle_agents = if swarm_state == SwarmState::Running {
            state.max_agents.saturating_sub(active_agents)
        } else {
            0
        };

        Ok(SwarmStats {
            state: swarm_state,
            max_agents: state.max_agents,
            active_agents,
            idle_agents,
            tasks_processed: queue_stats.completed as u64,
            tasks_failed: queue_stats.failed as u64,
        })
    }

    /// Check if swarm is running
    pub async fn is_running(&self) -> bool {
        if let Ok(state) = Self::read_state() {
            let is_running_state = matches!(state.state.as_str(), "Running" | "Starting");
            let process_alive = state.pid.map_or(false, Self::is_process_alive);
            is_running_state && process_alive
        } else {
            false
        }
    }
}

impl Default for SwarmService {
    fn default() -> Self {
        Self::new()
    }
}

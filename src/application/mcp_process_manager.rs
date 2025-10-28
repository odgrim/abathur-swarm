//! MCP Server Process Manager
//!
//! Manages the lifecycle of MCP server child processes (memory and task queue servers).
//! Starts them alongside the swarm orchestrator and ensures graceful shutdown.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, Command};
use tracing::{error, info, warn};

/// MCP server process manager
///
/// Manages MCP server child processes, ensuring they start and stop
/// in coordination with the swarm orchestrator.
pub struct McpProcessManager {
    memory_server: Option<Child>,
    task_server: Option<Child>,
    db_path: PathBuf,
}

impl McpProcessManager {
    /// Create a new MCP process manager
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path to the SQLite database file
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            memory_server: None,
            task_server: None,
            db_path,
        }
    }

    /// Start both MCP servers
    ///
    /// Spawns child processes for memory and task queue MCP servers.
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting MCP servers");

        // Determine binary paths
        let memory_bin = Self::find_binary("abathur-mcp-memory")?;
        let task_bin = Self::find_binary("abathur-mcp-tasks")?;

        // Start memory server
        info!(path = ?memory_bin, "Starting memory MCP server");
        let memory_child = Command::new(&memory_bin)
            .arg("--db-path")
            .arg(&self.db_path)
            .env("RUST_LOG", "info")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn memory MCP server")?;

        self.memory_server = Some(memory_child);
        info!("Memory MCP server started");

        // Start task queue server
        info!(path = ?task_bin, "Starting task queue MCP server");
        let task_child = Command::new(&task_bin)
            .arg("--db-path")
            .arg(&self.db_path)
            .env("RUST_LOG", "info")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn task queue MCP server")?;

        self.task_server = Some(task_child);
        info!("Task queue MCP server started");

        info!("All MCP servers started successfully");

        Ok(())
    }

    /// Stop all MCP servers
    ///
    /// Sends SIGTERM to all child processes and waits for them to exit.
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping MCP servers");

        // Stop memory server
        if let Some(mut child) = self.memory_server.take() {
            info!("Stopping memory MCP server");

            // Try graceful shutdown first
            if let Err(e) = child.start_kill() {
                warn!(error = ?e, "Failed to send kill signal to memory server");
            }

            // Wait for exit with timeout
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                child.wait(),
            )
            .await
            {
                Ok(Ok(status)) => {
                    info!(?status, "Memory MCP server exited");
                }
                Ok(Err(e)) => {
                    error!(error = ?e, "Error waiting for memory server to exit");
                }
                Err(_) => {
                    warn!("Memory server shutdown timeout, forcing kill");
                    let _ = child.kill().await;
                }
            }
        }

        // Stop task queue server
        if let Some(mut child) = self.task_server.take() {
            info!("Stopping task queue MCP server");

            // Try graceful shutdown first
            if let Err(e) = child.start_kill() {
                warn!(error = ?e, "Failed to send kill signal to task server");
            }

            // Wait for exit with timeout
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                child.wait(),
            )
            .await
            {
                Ok(Ok(status)) => {
                    info!(?status, "Task queue MCP server exited");
                }
                Ok(Err(e)) => {
                    error!(error = ?e, "Error waiting for task server to exit");
                }
                Err(_) => {
                    warn!("Task server shutdown timeout, forcing kill");
                    let _ = child.kill().await;
                }
            }
        }

        info!("All MCP servers stopped");

        Ok(())
    }

    /// Check if MCP servers are running
    pub fn is_running(&self) -> bool {
        self.memory_server.is_some() || self.task_server.is_some()
    }

    /// Find the binary path for an MCP server
    ///
    /// Searches in:
    /// 1. target/release/ (production builds)
    /// 2. target/debug/ (development builds)
    fn find_binary(name: &str) -> Result<PathBuf> {
        // Try release build first
        let release_path = PathBuf::from("target/release").join(name);
        if release_path.exists() {
            return Ok(release_path);
        }

        // Try debug build
        let debug_path = PathBuf::from("target/debug").join(name);
        if debug_path.exists() {
            return Ok(debug_path);
        }

        anyhow::bail!(
            "MCP server binary '{}' not found in target/release or target/debug. \
             Please build the project first with 'cargo build --release'",
            name
        )
    }
}

impl Drop for McpProcessManager {
    fn drop(&mut self) {
        // Ensure child processes are killed when manager is dropped
        if let Some(mut child) = self.memory_server.take() {
            let _ = child.start_kill();
        }
        if let Some(mut child) = self.task_server.take() {
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_binary_error_message() {
        // Test that find_binary returns a helpful error when binary not found
        let result = McpProcessManager::find_binary("nonexistent-binary");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found"));
        assert!(err_msg.contains("cargo build"));
    }
}

//! Worktree CLI commands.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::sqlite::{SqliteWorktreeRepository, initialize_database};
use crate::cli::id_resolver::{resolve_task_id, resolve_worktree_id};
use crate::cli::output::{output, CommandOutput};
use crate::domain::models::WorktreeStatus;
use crate::services::{WorktreeConfig, WorktreeService, WorktreeStats};

#[derive(Args, Debug)]
pub struct WorktreeArgs {
    #[command(subcommand)]
    pub command: WorktreeCommands,
}

#[derive(Subcommand, Debug)]
pub enum WorktreeCommands {
    /// Create a worktree for a task
    Create {
        /// Task ID
        task_id: String,
        /// Base ref (branch/commit to branch from)
        #[arg(short, long)]
        base_ref: Option<String>,
    },
    /// List worktrees
    List {
        /// Filter by status
        #[arg(short, long)]
        status: Option<String>,
        /// Show only active worktrees
        #[arg(long)]
        active: bool,
    },
    /// Show worktree details
    Show {
        /// Task ID or worktree ID
        id: String,
    },
    /// Mark worktree as completed
    Complete {
        /// Task ID
        task_id: String,
    },
    /// Merge a worktree back to base branch
    Merge {
        /// Task ID
        task_id: String,
    },
    /// Cleanup a worktree
    Cleanup {
        /// Worktree ID
        id: String,
    },
    /// Cleanup all eligible worktrees
    CleanupAll,
    /// Sync database with filesystem
    Sync,
    /// Show worktree statistics
    Stats,
}

#[derive(Debug, serde::Serialize)]
pub struct WorktreeOutput {
    pub id: String,
    pub task_id: String,
    pub path: String,
    pub branch: String,
    pub base_ref: String,
    pub status: String,
    pub merge_commit: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
}

impl From<&crate::domain::models::Worktree> for WorktreeOutput {
    fn from(wt: &crate::domain::models::Worktree) -> Self {
        Self {
            id: wt.id.to_string(),
            task_id: wt.task_id.to_string(),
            path: wt.path.clone(),
            branch: wt.branch.clone(),
            base_ref: wt.base_ref.clone(),
            status: wt.status.as_str().to_string(),
            merge_commit: wt.merge_commit.clone(),
            error_message: wt.error_message.clone(),
            created_at: wt.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct WorktreeListOutput {
    pub worktrees: Vec<WorktreeOutput>,
    pub total: usize,
}

impl CommandOutput for WorktreeListOutput {
    fn to_human(&self) -> String {
        if self.worktrees.is_empty() {
            return "No worktrees found.".to_string();
        }

        let mut lines = vec![format!("Found {} worktree(s):\n", self.total)];
        lines.push(format!(
            "{:<12} {:<36} {:<12} {:<30}",
            "STATUS", "TASK ID", "BRANCH", "PATH"
        ));
        lines.push("-".repeat(90));

        for wt in &self.worktrees {
            lines.push(format!(
                "{:<12} {:<36} {:<12} {:<30}",
                wt.status,
                &wt.task_id[..8],
                truncate(&wt.branch, 12),
                truncate(&wt.path, 30),
            ));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct WorktreeDetailOutput {
    pub worktree: WorktreeOutput,
}

impl CommandOutput for WorktreeDetailOutput {
    fn to_human(&self) -> String {
        let wt = &self.worktree;
        let mut lines = vec![
            format!("Worktree: {}", wt.id),
            format!("Task ID: {}", wt.task_id),
            format!("Status: {}", wt.status),
            format!("Path: {}", wt.path),
            format!("Branch: {}", wt.branch),
            format!("Base Ref: {}", wt.base_ref),
            format!("Created: {}", wt.created_at),
        ];

        if let Some(ref commit) = wt.merge_commit {
            lines.push(format!("Merge Commit: {}", commit));
        }

        if let Some(ref error) = wt.error_message {
            lines.push(format!("Error: {}", error));
        }

        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct WorktreeActionOutput {
    pub success: bool,
    pub message: String,
    pub worktree: Option<WorktreeOutput>,
}

impl CommandOutput for WorktreeActionOutput {
    fn to_human(&self) -> String {
        self.message.clone()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct WorktreeStatsOutput {
    pub creating: u64,
    pub active: u64,
    pub completed: u64,
    pub merging: u64,
    pub merged: u64,
    pub failed: u64,
    pub removed: u64,
    pub total: u64,
    pub active_total: u64,
}

impl From<WorktreeStats> for WorktreeStatsOutput {
    fn from(s: WorktreeStats) -> Self {
        Self {
            creating: s.creating,
            active: s.active,
            completed: s.completed,
            merging: s.merging,
            merged: s.merged,
            failed: s.failed,
            removed: s.removed,
            total: s.total(),
            active_total: s.active_count(),
        }
    }
}

impl CommandOutput for WorktreeStatsOutput {
    fn to_human(&self) -> String {
        let mut lines = vec!["Worktree Statistics:".to_string()];
        lines.push(format!("  Creating:   {}", self.creating));
        lines.push(format!("  Active:     {}", self.active));
        lines.push(format!("  Completed:  {}", self.completed));
        lines.push(format!("  Merging:    {}", self.merging));
        lines.push(format!("  Merged:     {}", self.merged));
        lines.push(format!("  Failed:     {}", self.failed));
        lines.push(format!("  Removed:    {}", self.removed));
        lines.push("  ------------".to_string());
        lines.push(format!("  Total:      {}", self.total));
        lines.push(format!("  Active:     {}", self.active_total));
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct SyncOutput {
    pub activated: u64,
    pub marked_removed: u64,
}

impl CommandOutput for SyncOutput {
    fn to_human(&self) -> String {
        format!(
            "Sync complete: {} activated, {} marked as removed",
            self.activated, self.marked_removed
        )
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: WorktreeArgs, json_mode: bool) -> Result<()> {
    let pool = initialize_database("sqlite:.abathur/abathur.db")
        .await
        .context("Failed to initialize database. Run 'abathur init' first.")?;

    let repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));
    let config = WorktreeConfig::default();
    let service = WorktreeService::new(repo, config);

    match args.command {
        WorktreeCommands::Create { task_id, base_ref } => {
            let task_uuid = resolve_task_id(&pool, &task_id).await?;

            let worktree = service.create_worktree(task_uuid, base_ref.as_deref()).await?;

            let out = WorktreeActionOutput {
                success: true,
                message: format!("Worktree created at: {}", worktree.path),
                worktree: Some(WorktreeOutput::from(&worktree)),
            };
            output(&out, json_mode);
        }

        WorktreeCommands::List { status, active } => {
            let worktrees = if active {
                service.list_active().await?
            } else if let Some(status_str) = status {
                let status = WorktreeStatus::from_str(&status_str)
                    .ok_or_else(|| anyhow::anyhow!("Invalid status: {}", status_str))?;
                service.list_by_status(status).await?
            } else {
                service.list_active().await?
            };

            let out = WorktreeListOutput {
                total: worktrees.len(),
                worktrees: worktrees.iter().map(WorktreeOutput::from).collect(),
            };
            output(&out, json_mode);
        }

        WorktreeCommands::Show { id } => {
            let uuid = resolve_worktree_id(&pool, &id).await?;
            let worktree = service.get_worktree_for_task(uuid).await?
                .or(service.get_worktree(uuid).await?);

            match worktree {
                Some(wt) => {
                    let out = WorktreeDetailOutput {
                        worktree: WorktreeOutput::from(&wt),
                    };
                    output(&out, json_mode);
                }
                None => {
                    let out = WorktreeActionOutput {
                        success: false,
                        message: format!("Worktree not found: {}", id),
                        worktree: None,
                    };
                    output(&out, json_mode);
                }
            }
        }

        WorktreeCommands::Complete { task_id } => {
            let task_uuid = resolve_task_id(&pool, &task_id).await?;

            let worktree = service.complete_worktree(task_uuid).await?;

            let out = WorktreeActionOutput {
                success: true,
                message: format!("Worktree marked as completed: {}", worktree.path),
                worktree: Some(WorktreeOutput::from(&worktree)),
            };
            output(&out, json_mode);
        }

        WorktreeCommands::Merge { task_id } => {
            let task_uuid = resolve_task_id(&pool, &task_id).await?;

            let worktree = service.merge_worktree(task_uuid).await?;

            let out = WorktreeActionOutput {
                success: true,
                message: format!(
                    "Worktree merged: {} -> {} (commit: {})",
                    worktree.branch,
                    worktree.base_ref,
                    worktree.merge_commit.as_deref().unwrap_or("unknown")
                ),
                worktree: Some(WorktreeOutput::from(&worktree)),
            };
            output(&out, json_mode);
        }

        WorktreeCommands::Cleanup { id } => {
            let uuid = resolve_worktree_id(&pool, &id).await?;

            service.cleanup_worktree(uuid).await?;

            let out = WorktreeActionOutput {
                success: true,
                message: format!("Worktree cleaned up: {}", id),
                worktree: None,
            };
            output(&out, json_mode);
        }

        WorktreeCommands::CleanupAll => {
            let cleaned = service.cleanup_all().await?;

            let out = WorktreeActionOutput {
                success: true,
                message: format!("Cleaned up {} worktree(s)", cleaned),
                worktree: None,
            };
            output(&out, json_mode);
        }

        WorktreeCommands::Sync => {
            let (activated, removed) = service.sync_with_filesystem().await?;

            let out = SyncOutput {
                activated,
                marked_removed: removed,
            };
            output(&out, json_mode);
        }

        WorktreeCommands::Stats => {
            let stats = service.get_stats().await?;

            let out = WorktreeStatsOutput::from(stats);
            output(&out, json_mode);
        }
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

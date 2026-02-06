//! Implementation of the `abathur init` command.

use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::adapters::sqlite::initialize_database;
use crate::cli::output::{output, CommandOutput};

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Force reinitialization even if already initialized
    #[arg(long, short)]
    pub force: bool,

    /// Target directory (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

#[derive(Debug, serde::Serialize)]
pub struct InitOutput {
    pub success: bool,
    pub message: String,
    pub initialized_path: PathBuf,
    pub directories_created: Vec<String>,
    pub database_initialized: bool,
    pub agents_copied: usize,
}

impl CommandOutput for InitOutput {
    fn to_human(&self) -> String {
        let mut lines = vec![self.message.clone()];
        if !self.directories_created.is_empty() {
            lines.push("\nCreated directories:".to_string());
            for dir in &self.directories_created {
                lines.push(format!("  - {}", dir));
            }
        }
        if self.database_initialized {
            lines.push("\nDatabase initialized at .abathur/abathur.db".to_string());
        }
        if self.agents_copied > 0 {
            lines.push(format!("\nCopied {} baseline agent(s)", self.agents_copied));
        }
        lines.join("\n")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

pub async fn execute(args: InitArgs, json_mode: bool) -> Result<()> {
    let target_path = if args.path.is_absolute() {
        args.path.clone()
    } else {
        std::env::current_dir().context("Failed to get current directory")?.join(&args.path)
    };

    let abathur_dir = target_path.join(".abathur");
    let claude_dir = target_path.join(".claude");

    // Check if already initialized
    if abathur_dir.exists() && !args.force {
        let output_data = InitOutput {
            success: false,
            message: "Project already initialized. Use --force to reinitialize.".to_string(),
            initialized_path: target_path,
            directories_created: vec![],
            database_initialized: false,
            agents_copied: 0,
        };
        output(&output_data, json_mode);
        return Ok(());
    }

    // If forcing, remove existing
    if args.force && abathur_dir.exists() {
        fs::remove_dir_all(&abathur_dir).await.context("Failed to remove existing .abathur directory")?;
    }

    let mut directories_created = vec![];

    // Create directories
    let dirs = [
        abathur_dir.clone(),
        abathur_dir.join("worktrees"),
        abathur_dir.join("logs"),
        claude_dir.clone(),
        claude_dir.join("agents"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            fs::create_dir_all(dir).await.with_context(|| format!("Failed to create {:?}", dir))?;
            let relative = dir.strip_prefix(&target_path).unwrap_or(dir).to_string_lossy().to_string();
            directories_created.push(relative);
        }
    }

    // Initialize database
    let db_path = abathur_dir.join("abathur.db");
    let db_url = format!("sqlite:{}", db_path.display());
    initialize_database(&db_url).await.context("Failed to initialize database")?;

    // Copy agents if source exists
    let agents_copied = copy_baseline_agents(&target_path).await.unwrap_or(0);

    let output_data = InitOutput {
        success: true,
        message: if args.force {
            "Project reinitialized successfully.".to_string()
        } else {
            "Project initialized successfully.".to_string()
        },
        initialized_path: target_path,
        directories_created,
        database_initialized: true,
        agents_copied,
    };

    output(&output_data, json_mode);
    Ok(())
}

async fn copy_baseline_agents(target_path: &Path) -> Result<usize> {
    let target_agents = target_path.join(".claude").join("agents");

    // Check if we're in the source repo (has agent definitions)
    let source_agents = target_path.join(".claude").join("agents");
    if source_agents.exists() {
        let overmind = source_agents.join("overmind.md");
        if overmind.exists() {
            // Already has agents, don't copy
            return Ok(0);
        }
    }

    // Try to find source agents from executable location
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            if let Some(parent) = exe_dir.parent() {
                let source = parent.join(".claude").join("agents");
                if source.exists() {
                    return copy_agents_recursive(&source, &target_agents).await;
                }
            }
        }
    }

    Ok(0)
}

async fn copy_agents_recursive(source: &Path, target: &Path) -> Result<usize> {
    let mut count = 0;
    let mut entries = fs::read_dir(source).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let file_name = entry.file_name();
        let target_path = target.join(&file_name);

        if path.is_dir() {
            if !target_path.exists() {
                fs::create_dir_all(&target_path).await?;
            }
            count += Box::pin(copy_agents_recursive(&path, &target_path)).await?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            fs::copy(&path, &target_path).await?;
            count += 1;
        }
    }

    Ok(count)
}

//! Implementation of the `abathur init` command.

use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::adapters::sqlite::initialize_database;
use crate::cli::output::{output, CommandOutput};
use crate::domain::models::workflow_template::DEFAULT_WORKFLOW_YAMLS;
use crate::services::config::Config;
use crate::ABATHUR_ALLOWED_TOOLS;

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
    pub workflows_written: Vec<String>,
    pub workflows_skipped: Vec<String>,
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
        if !self.workflows_written.is_empty() {
            lines.push("\nScaffolded workflows:".to_string());
            for wf in &self.workflows_written {
                lines.push(format!("  - {}", wf));
            }
        }
        if !self.workflows_skipped.is_empty() {
            lines.push("\nSkipped existing workflow files:".to_string());
            for wf in &self.workflows_skipped {
                lines.push(format!("  - {}", wf));
            }
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
            workflows_written: vec![],
            workflows_skipped: vec![],
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

    // Merge abathur MCP config into .claude/settings.json
    merge_claude_settings(&target_path).await.context("Failed to merge .claude/settings.json")?;

    // Scaffold default workflow YAMLs (skip any that already exist).
    let workflows_subdir = Config::default().workflows_dir;
    let workflows_dir = target_path.join(&workflows_subdir);
    let (workflows_written, workflows_skipped) =
        scaffold_default_workflows(&workflows_dir).await
            .with_context(|| format!("Failed to scaffold workflows in {:?}", workflows_dir))?;
    for wf in &workflows_skipped {
        tracing::info!(workflow = %wf, "skipped existing workflow file");
    }

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
        agents_copied: 0,
        workflows_written,
        workflows_skipped,
    };

    output(&output_data, json_mode);
    Ok(())
}

/// Write each embedded default workflow YAML into `dir` unless a file with the
/// same name is already there. Returns `(written, skipped)` filenames.
async fn scaffold_default_workflows(dir: &Path) -> Result<(Vec<String>, Vec<String>)> {
    fs::create_dir_all(dir).await
        .with_context(|| format!("Failed to create workflows directory {:?}", dir))?;

    let mut written = Vec::new();
    let mut skipped = Vec::new();
    for (name, contents) in DEFAULT_WORKFLOW_YAMLS {
        let filename = format!("{name}.yaml");
        let path = dir.join(&filename);
        if path.exists() {
            skipped.push(filename);
            continue;
        }
        fs::write(&path, contents).await
            .with_context(|| format!("Failed to write {:?}", path))?;
        written.push(filename);
    }
    Ok((written, skipped))
}


async fn merge_claude_settings(target_path: &Path) -> Result<()> {
    let settings_path = target_path.join(".claude").join("settings.json");

    // Read existing or start fresh
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path).await?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let map = settings.as_object_mut().expect("settings must be an object");

    // Merge mcpServers
    let servers = map
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("mcpServers must be an object");
    servers.remove("abathur-memory"); // legacy
    servers.remove("abathur-tasks"); // legacy
    servers.insert(
        "abathur".into(),
        serde_json::json!({
            "command": "abathur",
            "args": ["mcp", "stdio", "--db-path", "abathur.db"]
        }),
    );

    // Merge permissions.allowedTools
    let permissions = map
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("permissions must be an object");
    let tools = permissions
        .entry("allowedTools")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .expect("allowedTools must be an array");
    for tool in ABATHUR_ALLOWED_TOOLS {
        let val = serde_json::Value::String(tool.to_string());
        if !tools.contains(&val) {
            tools.push(val);
        }
    }

    // Write back (pretty-printed)
    let content = serde_json::to_string_pretty(&settings)?;
    fs::write(&settings_path, format!("{content}\n")).await?;
    Ok(())
}


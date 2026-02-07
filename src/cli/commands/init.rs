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

    // Write baseline agent definitions to disk
    let agents_copied = write_baseline_agents(&target_path).await.unwrap_or(0);

    // Merge abathur MCP config into .claude/settings.json
    merge_claude_settings(&target_path).await.context("Failed to merge .claude/settings.json")?;

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

const ABATHUR_TOOLS: &[&str] = &[
    "mcp__abathur__task_submit",
    "mcp__abathur__task_list",
    "mcp__abathur__task_get",
    "mcp__abathur__task_update_status",
    "mcp__abathur__agent_create",
    "mcp__abathur__agent_list",
    "mcp__abathur__agent_get",
    "mcp__abathur__memory_search",
    "mcp__abathur__memory_store",
    "mcp__abathur__memory_get",
    "mcp__abathur__goals_list",
];

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
    for tool in ABATHUR_TOOLS {
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

async fn write_baseline_agents(target_path: &Path) -> Result<usize> {
    use crate::domain::models::{AgentDefinition, specialist_templates};

    let target_agents = target_path.join(".claude").join("agents");

    // Don't overwrite existing agent definitions
    let overmind_path = target_agents.join("overmind.md");
    if overmind_path.exists() {
        return Ok(0);
    }

    // Generate agent definitions from hardcoded templates
    let baseline = specialist_templates::create_baseline_agents();
    let mut count = 0;

    for template in &baseline {
        let def = AgentDefinition::from_template(template);
        let file_path = target_agents.join(format!("{}.md", template.name));
        fs::write(&file_path, def.to_markdown()).await
            .with_context(|| format!("Failed to write agent definition {:?}", file_path))?;
        count += 1;
    }

    Ok(count)
}

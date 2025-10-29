//! Abathur setup and initialization infrastructure
//!
//! Handles project initialization including:
//! - Configuration directory creation
//! - Default config file creation
//! - Database migrations
//! - Template repository cloning
//! - Agent template installation
//! - MCP server configuration

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Default configuration template content
const DEFAULT_CONFIG_TEMPLATE: &str = r#"# Abathur Configuration
# Override settings by editing this file or setting environment variables
# with ABATHUR_ prefix
#
# Example environment variables:
#   export ABATHUR_MAX_AGENTS=20
#   export ABATHUR_RATE_LIMIT__REQUESTS_PER_SECOND=15.0
#   export ABATHUR_DATABASE__PATH=/custom/path/abathur.db
#   export ABATHUR_LOGGING__LEVEL=debug

# Maximum concurrent agents (1-100)
max_agents: 10

# Database configuration
database:
  # Path to SQLite database file (project-local)
  path: ".abathur/abathur.db"

  # Maximum number of database connections in pool
  max_connections: 10

# Logging configuration
logging:
  # Log level: trace, debug, info, warn, error
  level: "info"

  # Log format: json, pretty
  format: "json"

  # Number of days to retain logs
  retention_days: 30

# Claude API rate limiting
rate_limit:
  # Requests per second allowed
  requests_per_second: 10.0

  # Burst size for token bucket algorithm
  burst_size: 20

# Retry policy for transient failures
retry:
  # Maximum number of retry attempts
  max_retries: 3

  # Initial backoff delay in milliseconds
  initial_backoff_ms: 10000

  # Maximum backoff delay in milliseconds
  max_backoff_ms: 300000

# MCP (Model Context Protocol) server configurations
mcp_servers:
  - name: "memory"
    command: "npx"
    args:
      - "-y"
      - "@modelcontextprotocol/server-memory"
    env: {}

  - name: "github"
    command: "npx"
    args:
      - "-y"
      - "@modelcontextprotocol/server-github"
    env: {}
"#;

/// Setup paths and directories
pub struct SetupPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub database_file: PathBuf,
    pub agents_dir: PathBuf,
}

impl SetupPaths {
    /// Get setup paths for the current directory
    pub fn new() -> Result<Self> {
        let current_dir = std::env::current_dir()
            .context("Failed to get current directory")?;
        let config_dir = current_dir.join(".abathur");

        Ok(Self {
            config_file: config_dir.join("config.yaml"),
            database_file: config_dir.join("abathur.db"),
            agents_dir: config_dir.join("agents"),
            config_dir,
        })
    }

    /// Check if Abathur is already initialized
    pub fn is_initialized(&self) -> bool {
        self.config_file.exists()
            && self.database_file.exists()
            && self.agents_dir.exists()
    }
}

/// Create the configuration directory
pub fn create_config_dir(paths: &SetupPaths, force: bool) -> Result<()> {
    if paths.config_dir.exists() && !force {
        return Ok(());
    }

    fs::create_dir_all(&paths.config_dir)
        .context("Failed to create config directory")?;

    Ok(())
}

/// Create the default configuration file
pub fn create_config_file(paths: &SetupPaths, force: bool) -> Result<()> {
    if paths.config_file.exists() && !force {
        return Ok(());
    }

    fs::write(&paths.config_file, DEFAULT_CONFIG_TEMPLATE)
        .context("Failed to write config file")?;

    Ok(())
}

/// Run database migrations
pub async fn run_migrations(paths: &SetupPaths, force: bool) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = paths.database_file.parent() {
        fs::create_dir_all(parent)
            .context("Failed to create database directory")?;
    }

    let db_url = format!("sqlite:{}?mode=rwc", paths.database_file.display());

    // Check if database exists
    let db_exists = paths.database_file.exists();

    if db_exists && !force {
        return Ok(());
    }

    // Create database connection
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .context("Failed to connect to database")?;

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run migrations")?;

    pool.close().await;

    Ok(())
}

/// Clone template repository from GitHub
pub fn clone_template_repo(repo_url: &str, force: bool) -> Result<PathBuf> {
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?;

    let template_dir = current_dir.join("template");

    // Check if template directory already exists
    if template_dir.exists() {
        if force {
            fs::remove_dir_all(&template_dir)
                .context("Failed to remove existing template directory")?;
        } else {
            return Ok(template_dir);
        }
    }

    // Clone the repository using git command
    let output = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1") // Shallow clone to save bandwidth
        .arg(repo_url)
        .arg(&template_dir)
        .output()
        .context("Failed to execute git clone command. Is git installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to clone template repository: {}", stderr);
    }

    // Remove .git directory from cloned template
    let git_dir = template_dir.join(".git");
    if git_dir.exists() {
        fs::remove_dir_all(&git_dir)
            .context("Failed to remove .git directory from template")?;
    }

    Ok(template_dir)
}

/// Copy agent templates from template directory to .abathur/agents
pub fn copy_agent_templates(paths: &SetupPaths, template_dir: &PathBuf, force: bool) -> Result<()> {
    let template_agents_dir = template_dir.join(".claude/agents");

    // Check if template directory exists
    if !template_agents_dir.exists() {
        return Ok(());
    }

    // Create target agents directory
    if !paths.agents_dir.exists() || force {
        fs::create_dir_all(&paths.agents_dir)
            .context("Failed to create agents directory")?;
    }

    // Copy agent templates recursively
    copy_dir_recursive(&template_agents_dir, &paths.agents_dir, force)?;

    Ok(())
}

/// Recursively copy directory contents
fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf, force: bool) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dst.join(&file_name);

        if path.is_dir() {
            if !dest_path.exists() || force {
                fs::create_dir_all(&dest_path)?;
            }
            copy_dir_recursive(&path, &dest_path, force)?;
        } else if path.is_file() {
            if !dest_path.exists() || force {
                fs::copy(&path, &dest_path)
                    .with_context(|| format!("Failed to copy {} to {}", path.display(), dest_path.display()))?;
            }
        }
    }
    Ok(())
}

/// Merge Abathur MCP server configuration into Claude Code's MCP settings
pub fn merge_mcp_config(template_dir: &PathBuf, force: bool) -> Result<()> {
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?;

    let template_mcp_file = template_dir.join(".mcp.json");

    // Check if template MCP file exists
    if !template_mcp_file.exists() {
        return Ok(());
    }

    // Read template MCP configuration
    let template_content = fs::read_to_string(&template_mcp_file)
        .context("Failed to read template MCP configuration")?;

    // Replace placeholders with actual paths
    let project_root = current_dir.to_string_lossy();
    let configured_content = template_content.replace("{{ABATHUR_PROJECT_ROOT}}", &project_root);

    let template_mcp: Value = serde_json::from_str(&configured_content)
        .context("Failed to parse template MCP configuration")?;

    // Determine project-local MCP config path (repo root)
    let mcp_config_path = current_dir.join(".mcp.json");

    // Create parent directory if it doesn't exist
    if let Some(parent) = mcp_config_path.parent() {
        fs::create_dir_all(parent)
            .context("Failed to create Claude Code config directory")?;
    }

    // Read existing MCP config or create new one
    let mut existing_mcp: Value = if mcp_config_path.exists() {
        let existing_content = fs::read_to_string(&mcp_config_path)
            .context("Failed to read existing MCP configuration")?;
        serde_json::from_str(&existing_content)
            .context("Failed to parse existing MCP configuration")?
    } else {
        json!({ "mcpServers": {} })
    };

    // Merge configurations
    if let (Some(existing_servers), Some(template_servers)) = (
        existing_mcp.get_mut("mcpServers").and_then(|v| v.as_object_mut()),
        template_mcp.get("mcpServers").and_then(|v| v.as_object())
    ) {
        for (key, value) in template_servers {
            if !existing_servers.contains_key(key) || force {
                existing_servers.insert(key.clone(), value.clone());
            }
        }
    }

    // Write merged configuration
    let merged_content = serde_json::to_string_pretty(&existing_mcp)
        .context("Failed to serialize MCP configuration")?;

    fs::write(&mcp_config_path, merged_content)
        .context("Failed to write MCP configuration")?;

    Ok(())
}

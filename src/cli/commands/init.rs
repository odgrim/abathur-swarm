//! Init command handler
//!
//! Initializes Abathur by:
//! - Creating configuration directory (.abathur)
//! - Copying default config template
//! - Running database migrations
//! - Cloning template repository (if needed)
//! - Copying agent templates from template directory

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

/// Get the Abathur config directory path (project-local)
fn get_config_dir() -> Result<PathBuf> {
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?;
    Ok(current_dir.join(".abathur"))
}

/// Get the config file path
fn get_config_file_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join("config.yaml"))
}

/// Get the database file path
fn get_database_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join("abathur.db"))
}

/// Check if Abathur is already initialized
fn is_initialized() -> Result<bool> {
    let config_file = get_config_file_path()?;
    let db_file = get_database_path()?;
    let agents_dir = get_config_dir()?.join("agents");
    Ok(config_file.exists() && db_file.exists() && agents_dir.exists())
}

/// Create the configuration directory
fn create_config_dir(force: bool) -> Result<()> {
    let config_dir = get_config_dir()?;

    if config_dir.exists() && !force {
        println!("✓ Config directory already exists: {}", config_dir.display());
        return Ok(());
    }

    fs::create_dir_all(&config_dir)
        .context("Failed to create config directory")?;

    println!("✓ Created config directory: {}", config_dir.display());
    Ok(())
}

/// Copy the default configuration template
fn create_config_file(force: bool) -> Result<()> {
    let config_file = get_config_file_path()?;

    if config_file.exists() && !force {
        println!("✓ Config file already exists: {}", config_file.display());
        return Ok(());
    }

    fs::write(&config_file, DEFAULT_CONFIG_TEMPLATE)
        .context("Failed to write config file")?;

    println!("✓ Created config file: {}", config_file.display());
    Ok(())
}

/// Run database migrations
async fn run_migrations(force: bool) -> Result<()> {
    let db_path = get_database_path()?;

    // Ensure parent directory exists
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .context("Failed to create database directory")?;
    }

    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

    // Check if database exists
    let db_exists = db_path.exists();

    if db_exists && !force {
        println!("✓ Database already exists: {}", db_path.display());
    } else {
        // Create database connection
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&db_url)
            .await
            .context("Failed to connect to database")?;

        // Run migrations
        println!("Running database migrations...");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("Failed to run migrations")?;

        println!("✓ Database initialized: {}", db_path.display());

        pool.close().await;
    }

    Ok(())
}

/// Clone template repository from GitHub
fn clone_template_repo(repo_url: &str, force: bool) -> Result<()> {
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?;

    let template_dir = current_dir.join("template");

    // Check if template directory already exists
    if template_dir.exists() {
        if force {
            println!("Removing existing template directory...");
            fs::remove_dir_all(&template_dir)
                .context("Failed to remove existing template directory")?;
        } else {
            println!("✓ Template directory already exists: {}", template_dir.display());
            return Ok(());
        }
    }

    println!("Cloning template repository from {}...", repo_url);

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

    println!("✓ Cloned template repository to {}", template_dir.display());

    Ok(())
}

/// Copy agent templates from template directory to .abathur/agents
fn copy_agent_templates(force: bool) -> Result<()> {
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?;

    let template_agents_dir = current_dir.join("template/.claude/agents");
    let target_agents_dir = get_config_dir()?.join("agents");

    // Check if template directory exists
    if !template_agents_dir.exists() {
        println!("⚠ Template agents directory not found: {}", template_agents_dir.display());
        println!("  Skipping agent template installation");
        return Ok(());
    }

    // Create target agents directory
    if !target_agents_dir.exists() || force {
        fs::create_dir_all(&target_agents_dir)
            .context("Failed to create agents directory")?;
        println!("✓ Created agents directory: {}", target_agents_dir.display());
    }

    // Copy agent templates recursively
    copy_dir_recursive(&template_agents_dir, &target_agents_dir, force)?;

    println!("✓ Copied agent templates from {}", template_agents_dir.display());

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
fn merge_mcp_config(force: bool) -> Result<()> {
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?;

    let template_mcp_file = current_dir.join("template/.mcp.json");

    // Check if template MCP file exists
    if !template_mcp_file.exists() {
        println!("⚠ Template MCP configuration not found: {}", template_mcp_file.display());
        println!("  Skipping MCP server configuration");
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
            if existing_servers.contains_key(key) && !force {
                println!("✓ MCP server '{}' already configured (use --force to override)", key);
            } else {
                existing_servers.insert(key.clone(), value.clone());
                println!("✓ Added MCP server '{}'", key);
            }
        }
    }

    // Write merged configuration
    let merged_content = serde_json::to_string_pretty(&existing_mcp)
        .context("Failed to serialize MCP configuration")?;

    fs::write(&mcp_config_path, merged_content)
        .context("Failed to write MCP configuration")?;

    println!("✓ Merged MCP server configuration into {}", mcp_config_path.display());

    Ok(())
}

/// Handle init command
pub async fn handle_init(force: bool, template_repo: &str, skip_clone: bool, json_output: bool) -> Result<()> {
    if json_output {
        let output = json!({
            "status": "initializing",
            "force": force
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Initializing Abathur...");
        println!();
    }

    // Check if already initialized
    if !force && is_initialized()? {
        if json_output {
            let output = json!({
                "status": "already_initialized",
                "message": "Abathur is already initialized. Use --force to reinitialize."
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("✓ Abathur is already initialized!");
            println!();
            println!("Configuration: {}", get_config_file_path()?.display());
            println!("Database: {}", get_database_path()?.display());
            println!();
            println!("Use 'abathur init --force' to reinitialize.");
        }
        return Ok(());
    }

    // Step 1: Create config directory
    create_config_dir(force)?;

    // Step 2: Create config file
    create_config_file(force)?;

    // Step 3: Run migrations
    run_migrations(force).await?;

    // Step 4: Clone template repository (if not skipped)
    if !skip_clone {
        clone_template_repo(template_repo, force)?;
    } else {
        println!("⚠ Skipping template repository clone");
    }

    // Step 5: Copy agent templates
    copy_agent_templates(force)?;

    // Step 6: Merge MCP server configuration
    merge_mcp_config(force)?;

    if json_output {
        let output = json!({
            "status": "initialized",
            "config_dir": get_config_dir()?.display().to_string(),
            "config_file": get_config_file_path()?.display().to_string(),
            "database": get_database_path()?.display().to_string(),
            "agents_dir": get_config_dir()?.join("agents").display().to_string()
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!();
        println!("✓ Abathur initialized successfully!");
        println!();
        println!("Configuration: {}", get_config_file_path()?.display());
        println!("Database: {}", get_database_path()?.display());
        println!("Agents: {}", get_config_dir()?.join("agents").display());
        println!();
        println!("Next steps:");
        println!("  1. Edit your config file to customize settings");
        println!("  2. Set ANTHROPIC_API_KEY environment variable");
        println!("  3. Run 'abathur swarm start' to start the orchestrator");
    }

    Ok(())
}

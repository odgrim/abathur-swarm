//! Init command handler
//!
//! Thin adapter that delegates to infrastructure setup module.

use anyhow::{Context, Result};
use crate::domain::models::{Task, TaskSource};
use crate::domain::ports::TaskRepository;
use crate::infrastructure::config::ConfigLoader;
use crate::infrastructure::database::{DatabaseConnection, TaskRepositoryImpl};
use crate::infrastructure::setup;
use serde_json::json;

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

    // Get setup paths
    let paths = setup::SetupPaths::new()?;

    // Check if already initialized
    if !force && paths.is_initialized() {
        if json_output {
            let output = json!({
                "status": "already_initialized",
                "message": "Abathur is already initialized. Use --force to reinitialize."
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("✓ Abathur is already initialized!");
            println!();
            println!("Configuration: {}", paths.config_file.display());
            println!("Database: {}", paths.database_file.display());
            println!();
            println!("Use 'abathur init --force' to reinitialize.");
        }
        return Ok(());
    }

    // Step 1: Create config directory
    setup::create_config_dir(&paths, force)?;
    if !json_output {
        println!("✓ Created config directory: {}", paths.config_dir.display());
    }

    // Step 2: Create config file
    setup::create_config_file(&paths, force)?;
    if !json_output {
        println!("✓ Created config file: {}", paths.config_file.display());
    }

    // Step 3: Run migrations
    setup::run_migrations(&paths, force).await?;
    if !json_output {
        println!("✓ Database initialized: {}", paths.database_file.display());
    }

    // Step 3.5: Enqueue project-context-scanner task
    let config = ConfigLoader::load().context("Failed to load config")?;
    let database_url = format!("sqlite:{}", config.database.path);
    let db = DatabaseConnection::new(&database_url)
        .await
        .context("Failed to create database connection")?;
    let task_repo = TaskRepositoryImpl::new(db.pool().clone());

    // Create the project-context-scanner task with highest priority
    let mut scanner_task = Task::new(
        "Scan project context".to_string(),
        "Initial project scan to detect language, framework, and conventions.".to_string(),
    );
    scanner_task.agent_type = "project-context-scanner".to_string();
    scanner_task.priority = 10; // Highest priority - runs first
    scanner_task.source = TaskSource::Human;

    // Insert the task into the database
    task_repo.insert(&scanner_task)
        .await
        .context("Failed to enqueue project-context-scanner task")?;

    if !json_output {
        println!("✓ Enqueued project-context-scanner task (priority: 10)");
    }

    // Step 4: Clone template repository (if not skipped)
    let template_dir = if !skip_clone {
        let dir = setup::clone_template_repo(template_repo, force)?;
        if !json_output {
            println!("✓ Cloned template repository to {}", dir.display());
        }
        Some(dir)
    } else {
        if !json_output {
            println!("⚠ Skipping template repository clone");
        }
        None
    };

    // Step 5: Copy agent templates
    if let Some(ref template_dir) = template_dir {
        setup::copy_agent_templates(&paths, template_dir, force)?;
        if !json_output {
            println!("✓ Copied agent templates");
        }

        // Step 6: Copy hooks configuration
        setup::copy_hooks_config(&paths, template_dir, force)?;
        if !json_output {
            println!("✓ Copied hooks configuration");
        }

        // Step 7: Copy hook scripts
        setup::copy_hook_scripts(&paths, template_dir, force)?;
        if !json_output {
            println!("✓ Copied hook scripts");
        }

        // Step 8: Merge MCP server configuration
        setup::merge_mcp_config(template_dir, force)?;
        if !json_output {
            println!("✓ Merged MCP server configuration");
        }
    }

    if json_output {
        let output = json!({
            "status": "initialized",
            "config_dir": paths.config_dir.display().to_string(),
            "config_file": paths.config_file.display().to_string(),
            "database": paths.database_file.display().to_string(),
            "agents_dir": paths.agents_dir.display().to_string(),
            "hooks_file": paths.hooks_file.display().to_string(),
            "hooks_dir": paths.hooks_dir.display().to_string(),
            "initial_task": {
                "id": scanner_task.id.to_string(),
                "agent_type": "project-context-scanner",
                "priority": 10,
                "summary": "Scan project context"
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!();
        println!("✓ Abathur initialized successfully!");
        println!();
        println!("Configuration: {}", paths.config_file.display());
        println!("Database: {}", paths.database_file.display());
        println!("Agents: {}", paths.agents_dir.display());
        println!("Hooks: {}", paths.hooks_file.display());
        println!();
        println!("Initial task enqueued:");
        println!("  - project-context-scanner (priority: 10)");
        println!("  - Will run first when swarm starts");
        println!();
        println!("Next steps:");
        println!("  1. Edit your config file to customize settings");
        println!("  2. Set ANTHROPIC_API_KEY environment variable");
        println!("  3. Customize hooks in {} if needed", paths.hooks_file.display());
        println!("  4. Run 'abathur swarm start' to start the orchestrator");
    }

    Ok(())
}

//! Swarm command handlers
//!
//! Handlers for swarm orchestration commands including start, stop, and status.

use crate::application::{
    agent_executor::AgentExecutor, resource_monitor::ResourceMonitor,
    task_coordinator::TaskCoordinator, SwarmOrchestrator,
};
use crate::cli::service::{SwarmService, TaskQueueServiceAdapter};
use crate::infrastructure::config::ConfigLoader;
use crate::infrastructure::database::{AgentRepositoryImpl, MemoryRepositoryImpl, TaskRepositoryImpl};
use crate::infrastructure::mcp::DirectMcpClient;
use crate::services::{DependencyResolver, MemoryService, PriorityCalculator, TaskQueueService as TaskQueueServiceImpl};
use anyhow::{Context, Result};
use serde_json::json;
use std::sync::Arc;

/// Handle swarm start command
///
/// Starts the swarm orchestrator with the specified maximum number of agents.
pub async fn handle_start(
    _task_service: &TaskQueueServiceAdapter,
    max_agents: usize,
    json_output: bool,
) -> Result<()> {
    let swarm_service = SwarmService::new();

    // Check if database is initialized
    let db_path = std::env::current_dir()?.join(".abathur/abathur.db");
    let db_initialized = db_path.exists();

    // Attempt to start the swarm
    match swarm_service.start(max_agents).await {
        Ok(()) => {
            if json_output {
                let output = json!({
                    "status": "started",
                    "max_agents": max_agents,
                    "message": "Swarm orchestrator started successfully",
                    "database_initialized": db_initialized,
                    "log_file": ".abathur/swarm_daemon.log"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Starting swarm orchestrator with {} max agents...", max_agents);
                println!("Swarm orchestrator started successfully");
                println!();
                println!("Daemon logs are written to: .abathur/swarm_daemon.log");
                if !db_initialized {
                    println!();
                    println!("Note: Full orchestration requires database setup.");
                    println!("Run 'abathur init' to initialize Abathur.");
                }
            }
            Ok(())
        }
        Err(e) => {
            if json_output {
                let output = json!({
                    "status": "error",
                    "message": format!("{}", e),
                    "log_file": ".abathur/swarm_daemon.log"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Failed to start swarm orchestrator: {}", e);
                println!();
                println!("Check logs at: .abathur/swarm_daemon.log");
                println!();
                println!("To enable full swarm functionality:");
                println!("  1. Run 'abathur init' to initialize Abathur");
                println!("  2. Ensure ANTHROPIC_API_KEY environment variable is set");
            }
            Ok(()) // Don't fail the CLI, just inform the user
        }
    }
}

/// Handle swarm stop command
///
/// Gracefully stops the swarm orchestrator.
pub async fn handle_stop(
    _task_service: &TaskQueueServiceAdapter,
    json_output: bool,
) -> Result<()> {
    let swarm_service = SwarmService::new();

    match swarm_service.stop().await {
        Ok(()) => {
            if json_output {
                let output = json!({
                    "status": "stopped",
                    "message": "Swarm orchestrator stopped successfully"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Stopping swarm orchestrator...");
                println!("Swarm orchestrator stopped successfully");
            }
            Ok(())
        }
        Err(e) => {
            if json_output {
                let output = json!({
                    "status": "error",
                    "message": format!("{}", e)
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Failed to stop swarm orchestrator: {}", e);
            }
            Ok(()) // Don't fail the CLI
        }
    }
}

/// Handle swarm status command
///
/// Shows the current status of the swarm orchestrator.
pub async fn handle_status(
    task_service: &TaskQueueServiceAdapter,
    json_output: bool,
) -> Result<()> {
    let swarm_service = SwarmService::new();

    // Get swarm stats (includes task stats from database)
    let swarm_stats = swarm_service.get_stats(task_service).await?;

    // Get queue stats for detailed breakdown
    let queue_stats = task_service.get_queue_stats().await?;

    if json_output {
        let output = json!({
            "swarm": {
                "state": format!("{:?}", swarm_stats.state),
                "active_agents": swarm_stats.active_agents,
                "idle_agents": swarm_stats.idle_agents,
                "max_agents": swarm_stats.max_agents,
                "tasks_processed": swarm_stats.tasks_processed,
                "tasks_failed": swarm_stats.tasks_failed,
            },
            "queue": {
                "total": queue_stats.total,
                "pending": queue_stats.pending,
                "blocked": queue_stats.blocked,
                "ready": queue_stats.ready,
                "running": queue_stats.running,
                "completed": queue_stats.completed,
                "failed": queue_stats.failed,
                "cancelled": queue_stats.cancelled,
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Swarm Orchestrator Status");
        println!("========================");
        println!("State: {:?}", swarm_stats.state);
        println!("Active Agents: {}", swarm_stats.active_agents);
        println!("Idle Agents: {}", swarm_stats.idle_agents);
        println!("Max Agents: {}", swarm_stats.max_agents);
        println!("Tasks Processed: {}", swarm_stats.tasks_processed);
        println!("Tasks Failed: {}", swarm_stats.tasks_failed);
        println!();
        println!("Queue Statistics:");
        println!("  Total Tasks: {}", queue_stats.total);
        println!("  Pending: {}", queue_stats.pending);
        println!("  Blocked: {}", queue_stats.blocked);
        println!("  Ready: {}", queue_stats.ready);
        println!("  Running: {}", queue_stats.running);
        println!("  Completed: {}", queue_stats.completed);
        println!("  Failed: {}", queue_stats.failed);
        println!("  Cancelled: {}", queue_stats.cancelled);
    }

    Ok(())
}

/// Handle daemon mode - runs the actual SwarmOrchestrator
///
/// This function runs in the background process and initializes all dependencies.
pub async fn handle_daemon(max_agents: usize) -> Result<()> {
    // Initialize simple logging (eprintln for now)
    eprintln!("Starting Abathur swarm daemon with max_agents={}", max_agents);

    // Load configuration
    let config = ConfigLoader::load()
        .context("Failed to load configuration")?;

    eprintln!("Configuration loaded successfully");

    // Initialize database connection
    let db_path = std::path::PathBuf::from(&config.database.path);
    let db_path = if db_path.is_relative() {
        std::env::current_dir()?.join(db_path)
    } else {
        db_path
    };

    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

    eprintln!("Connecting to database: {}", db_path.display());

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(config.database.max_connections)
        .connect(&db_url)
        .await
        .context("Failed to connect to database")?;

    eprintln!("Database connection established");

    // Initialize repositories
    let task_repo = Arc::new(TaskRepositoryImpl::new(pool.clone()));
    let memory_repo = Arc::new(MemoryRepositoryImpl::new(pool.clone()));
    let _agent_repo = Arc::new(AgentRepositoryImpl::new(pool.clone()));

    // Initialize dependency resolver and priority calculator early
    let dependency_resolver = DependencyResolver::new();
    let priority_calc = PriorityCalculator::new();

    // Initialize services
    let task_queue_service: Arc<dyn crate::domain::ports::TaskQueueService> =
        Arc::new(TaskQueueServiceImpl::new(
            task_repo.clone(),
            dependency_resolver.clone(),
            priority_calc.clone(),
        ));

    let memory_service = Arc::new(MemoryService::new(memory_repo));

    // Initialize substrate registry from config
    eprintln!("Initializing LLM substrates...");
    let substrate_registry = Arc::new(
        crate::infrastructure::substrates::SubstrateRegistry::from_config(&config)
            .await
            .context("Failed to initialize substrate registry")?,
    );

    // Check if at least one substrate is healthy
    eprintln!("Checking substrate health...");
    if !substrate_registry.is_any_substrate_healthy().await {
        anyhow::bail!(
            "No healthy substrates available. Available substrates: {:?}\n\
             Please ensure at least one substrate is properly configured:\n\
             - For Claude Code: Install and authenticate the claude CLI\n\
             - For Anthropic API: Set ANTHROPIC_API_KEY environment variable",
            substrate_registry.available_substrates()
        );
    }

    eprintln!("Substrate registry initialized successfully");
    let health_status = substrate_registry.health_check_all().await;
    for (id, status) in health_status {
        eprintln!("  {} Substrate '{}': {:?}",
            if matches!(status, Ok(crate::domain::ports::HealthStatus::Healthy)) { "✓" } else { "✗" },
            id,
            status
        );
    }

    // Initialize direct MCP client for internal agents
    // This provides efficient in-process access to memory and task services
    // without spawning separate MCP server processes
    eprintln!("Initializing direct MCP client for agents...");
    let mcp_client: Arc<dyn crate::domain::ports::McpClient> = Arc::new(DirectMcpClient::new(
        memory_service,
        Arc::new(TaskQueueServiceImpl::new(
            task_repo.clone(),
            dependency_resolver.clone(),
            priority_calc.clone(),
        )),
    ));

    // Initialize agent metadata registry
    eprintln!("Loading agent metadata...");
    let agents_dir = std::env::current_dir()
        .context("Failed to get current directory")?
        .join(".claude/agents");

    let mut metadata_registry = crate::domain::models::AgentMetadataRegistry::new(&agents_dir);
    if agents_dir.exists() {
        if let Err(e) = metadata_registry.load_all() {
            eprintln!("Warning: Failed to load some agent metadata: {}", e);
        } else {
            eprintln!("Agent metadata loaded successfully");
        }
    } else {
        eprintln!("Warning: Agent directory not found at {}", agents_dir.display());
    }
    let agent_metadata_registry = Arc::new(std::sync::Mutex::new(metadata_registry));

    // Wrap for trait objects
    let dependency_resolver_arc = Arc::new(dependency_resolver);
    let priority_calc_arc: Arc<dyn crate::domain::ports::PriorityCalculator> =
        Arc::new(priority_calc);

    // Initialize application components
    let task_coordinator = Arc::new(TaskCoordinator::new(
        task_queue_service.clone(),
        dependency_resolver_arc,
        priority_calc_arc,
    ));

    // Create AgentExecutor with substrate registry and agent metadata
    let agent_executor = Arc::new(AgentExecutor::new(
        substrate_registry.clone(),
        mcp_client,
        agent_metadata_registry,
    ));

    // Create resource monitor with default limits (monitoring only, no enforcement)
    let resource_monitor = Arc::new(ResourceMonitor::new(
        crate::application::resource_monitor::ResourceLimits::default()
    ));

    eprintln!("All dependencies initialized, creating SwarmOrchestrator");

    // Create and start SwarmOrchestrator
    // Agents use DirectMcpClient for efficient in-process service access
    let mut orchestrator = SwarmOrchestrator::new(
        max_agents,
        task_coordinator,
        agent_executor,
        resource_monitor,
        config,
    );

    eprintln!("Starting SwarmOrchestrator...");

    orchestrator.start().await
        .context("Failed to start SwarmOrchestrator")?;

    eprintln!("SwarmOrchestrator started successfully");
    eprintln!("  - Agents use direct in-process service access (no separate MCP processes)");
    eprintln!("  - External clients (Claude Code) can spawn stdio MCP servers via .mcp.json");

    // Run forever until interrupted
    // The orchestrator runs its background tasks automatically
    tokio::signal::ctrl_c().await
        .context("Failed to listen for ctrl-c")?;

    eprintln!("Received shutdown signal, stopping SwarmOrchestrator");

    orchestrator.stop().await
        .context("Failed to stop SwarmOrchestrator")?;

    eprintln!("SwarmOrchestrator stopped successfully");

    Ok(())
}

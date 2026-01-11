//! Swarm orchestration CLI commands.

use anyhow::Result;
use clap::{Args, Subcommand};
use tokio::sync::mpsc;

use crate::services::{SwarmConfig, SwarmOrchestrator, SwarmEvent};

#[derive(Args, Debug)]
pub struct SwarmArgs {
    #[command(subcommand)]
    pub command: SwarmCommand,
}

#[derive(Subcommand, Debug)]
pub enum SwarmCommand {
    /// Run the swarm orchestrator
    Run {
        /// Maximum concurrent agents
        #[arg(long, default_value = "4")]
        max_agents: usize,

        /// Poll interval in milliseconds
        #[arg(long, default_value = "5000")]
        poll_interval_ms: u64,

        /// Run in dry-run mode (no actual execution)
        #[arg(long)]
        dry_run: bool,

        /// Stop after processing this many goals
        #[arg(long)]
        max_goals: Option<usize>,
    },
    /// Show current swarm status
    Status,
    /// List active goals and tasks
    Active,
    /// Show swarm configuration
    Config,
    /// Run a single tick (process one cycle)
    Tick,
}

pub async fn execute(args: SwarmArgs, json_mode: bool) -> Result<()> {
    match args.command {
        SwarmCommand::Run { max_agents, poll_interval_ms, dry_run, max_goals } => {
            run_swarm(max_agents, poll_interval_ms, dry_run, max_goals, json_mode).await
        }
        SwarmCommand::Status => show_status(json_mode).await,
        SwarmCommand::Active => show_active(json_mode).await,
        SwarmCommand::Config => show_config(json_mode).await,
        SwarmCommand::Tick => run_tick(json_mode).await,
    }
}

async fn run_swarm(
    max_agents: usize,
    poll_interval_ms: u64,
    dry_run: bool,
    _max_goals: Option<usize>,
    json_mode: bool,
) -> Result<()> {
    use crate::adapters::sqlite::{
        create_pool, SqliteGoalRepository, SqliteTaskRepository, SqliteWorktreeRepository,
        Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::SubstrateRegistry;
    use crate::domain::models::SubstrateType;
    use std::sync::Arc;

    // Initialize database
    let pool = create_pool("abathur.db", None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator.run_embedded_migrations(all_embedded_migrations()).await?;

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));

    // Get substrate (use mock for dry-run)
    let registry = SubstrateRegistry::new();
    let substrate: Arc<dyn crate::domain::ports::Substrate> = if dry_run {
        Arc::from(registry.create_by_type(SubstrateType::Mock))
    } else {
        Arc::from(registry.default_substrate())
    };

    let config = SwarmConfig {
        max_agents,
        poll_interval_ms,
        ..Default::default()
    };

    let orchestrator = SwarmOrchestrator::new(
        goal_repo,
        task_repo,
        worktree_repo,
        substrate,
        config.clone(),
    );

    if !json_mode {
        println!("Starting Abathur Swarm Orchestrator");
        println!("   Max agents: {}", max_agents);
        println!("   Poll interval: {}ms", poll_interval_ms);
        if dry_run {
            println!("   Mode: DRY RUN (using mock substrate)");
        }
        println!();
    }

    // Create event channel for monitoring
    let (event_tx, mut event_rx) = mpsc::channel::<SwarmEvent>(100);

    // Spawn event handler
    let event_handler = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match &event {
                SwarmEvent::Started => {
                    if !json_mode {
                        println!("Orchestrator started");
                    }
                }
                SwarmEvent::GoalStarted { goal_id, goal_name } => {
                    if !json_mode {
                        println!("Goal started: {} ({})", goal_name, goal_id);
                    }
                }
                SwarmEvent::GoalCompleted { goal_id } => {
                    if !json_mode {
                        println!("Goal completed: {}", goal_id);
                    }
                }
                SwarmEvent::GoalFailed { goal_id, error } => {
                    if !json_mode {
                        println!("Goal failed: {} - {}", goal_id, error);
                    }
                }
                SwarmEvent::TaskSpawned { task_id, task_title, agent_type } => {
                    if !json_mode {
                        println!("  Task started: {} ({}) [agent: {:?}]", task_title, task_id, agent_type);
                    }
                }
                SwarmEvent::TaskCompleted { task_id } => {
                    if !json_mode {
                        println!("  Task completed: {}", task_id);
                    }
                }
                SwarmEvent::TaskFailed { task_id, error } => {
                    if !json_mode {
                        println!("  Task failed: {} - {}", task_id, error);
                    }
                }
                SwarmEvent::StatusUpdate(stats) => {
                    if !json_mode && stats.active_goals > 0 {
                        println!("Status: {} active goals, {} running tasks",
                            stats.active_goals, stats.running_tasks);
                    }
                }
                SwarmEvent::Paused => {
                    if !json_mode {
                        println!("Orchestrator paused");
                    }
                }
                SwarmEvent::Resumed => {
                    if !json_mode {
                        println!("Orchestrator resumed");
                    }
                }
                SwarmEvent::Stopped => {
                    if !json_mode {
                        println!("Orchestrator stopped");
                    }
                    break;
                }
            }
        }
    });

    // Run the orchestrator
    let run_result = orchestrator.run(event_tx).await;

    // Wait for event handler to finish
    let _ = event_handler.await;

    match run_result {
        Ok(()) => {
            if !json_mode {
                println!("\nSwarm orchestrator completed successfully");
            }
            Ok(())
        }
        Err(e) => {
            if !json_mode {
                println!("\nSwarm orchestrator error: {}", e);
            }
            Err(e.into())
        }
    }
}

async fn show_status(json_mode: bool) -> Result<()> {
    use crate::adapters::sqlite::{
        create_pool, SqliteGoalRepository, SqliteTaskRepository, SqliteWorktreeRepository,
    };
    use std::sync::Arc;
    use crate::domain::models::{GoalStatus, TaskStatus, WorktreeStatus};
    use crate::domain::ports::{GoalRepository, GoalFilter, TaskRepository, WorktreeRepository};

    let pool = create_pool("abathur.db", None).await?;

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));

    // Get counts
    let active_goals = goal_repo.list(GoalFilter { status: Some(GoalStatus::Active), ..Default::default() }).await?.len();
    let pending_tasks = task_repo.list_by_status(TaskStatus::Pending).await?.len();
    let running_tasks = task_repo.list_by_status(TaskStatus::Running).await?.len();
    let active_worktrees = worktree_repo.list_by_status(WorktreeStatus::Active).await?.len();

    if json_mode {
        let output = serde_json::json!({
            "status": "idle",
            "active_goals": active_goals,
            "pending_tasks": pending_tasks,
            "running_tasks": running_tasks,
            "active_worktrees": active_worktrees
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Swarm Status");
        println!("============");
        println!("Active goals:     {}", active_goals);
        println!("Pending tasks:    {}", pending_tasks);
        println!("Running tasks:    {}", running_tasks);
        println!("Active worktrees: {}", active_worktrees);
    }

    Ok(())
}

async fn show_active(json_mode: bool) -> Result<()> {
    use crate::adapters::sqlite::{
        create_pool, SqliteGoalRepository, SqliteTaskRepository,
    };
    use std::sync::Arc;
    use crate::domain::models::{GoalStatus, TaskStatus};
    use crate::domain::ports::{GoalRepository, GoalFilter, TaskRepository};

    let pool = create_pool("abathur.db", None).await?;

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));

    let active_goals = goal_repo.list(GoalFilter { status: Some(GoalStatus::Active), ..Default::default() }).await?;
    let running_tasks = task_repo.list_by_status(TaskStatus::Running).await?;
    let pending_tasks = task_repo.list_by_status(TaskStatus::Pending).await?;

    if json_mode {
        let output = serde_json::json!({
            "active_goals": active_goals.iter().map(|g| serde_json::json!({
                "id": g.id.to_string(),
                "name": g.name,
                "priority": format!("{:?}", g.priority)
            })).collect::<Vec<_>>(),
            "running_tasks": running_tasks.iter().map(|t| serde_json::json!({
                "id": t.id.to_string(),
                "title": t.title,
                "agent": t.agent_type
            })).collect::<Vec<_>>(),
            "pending_tasks": pending_tasks.len()
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Active Goals ({}):", active_goals.len());
        for goal in &active_goals {
            println!("  {} - {} [{:?}]", goal.id, goal.name, goal.priority);
        }

        println!("\nRunning Tasks ({}):", running_tasks.len());
        for task in &running_tasks {
            println!("  {} - {} [agent: {:?}]", task.id, task.title, task.agent_type);
        }

        println!("\nPending Tasks: {}", pending_tasks.len());
    }

    Ok(())
}

async fn show_config(json_mode: bool) -> Result<()> {
    let config = SwarmConfig::default();

    if json_mode {
        let output = serde_json::json!({
            "max_agents": config.max_agents,
            "default_max_turns": config.default_max_turns,
            "use_worktrees": config.use_worktrees,
            "poll_interval_ms": config.poll_interval_ms,
            "goal_timeout_secs": config.goal_timeout_secs,
            "auto_retry": config.auto_retry,
            "max_task_retries": config.max_task_retries
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Swarm Configuration");
        println!("===================");
        println!("Max agents:         {}", config.max_agents);
        println!("Default max turns:  {}", config.default_max_turns);
        println!("Use worktrees:      {}", config.use_worktrees);
        println!("Poll interval (ms): {}", config.poll_interval_ms);
        println!("Goal timeout (s):   {}", config.goal_timeout_secs);
        println!("Auto-retry:         {}", config.auto_retry);
        println!("Max task retries:   {}", config.max_task_retries);
    }

    Ok(())
}

async fn run_tick(json_mode: bool) -> Result<()> {
    use crate::adapters::sqlite::{
        create_pool, SqliteGoalRepository, SqliteTaskRepository, SqliteWorktreeRepository,
        Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::SubstrateRegistry;
    use std::sync::Arc;

    let pool = create_pool("abathur.db", None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator.run_embedded_migrations(all_embedded_migrations()).await?;

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));

    let substrate: std::sync::Arc<dyn crate::domain::ports::Substrate> =
        std::sync::Arc::from(SubstrateRegistry::mock_substrate());

    let config = SwarmConfig::default();
    let orchestrator = SwarmOrchestrator::new(
        goal_repo,
        task_repo,
        worktree_repo,
        substrate,
        config,
    );

    let stats = orchestrator.tick().await?;

    if json_mode {
        let output = serde_json::json!({
            "active_goals": stats.active_goals,
            "pending_tasks": stats.pending_tasks,
            "running_tasks": stats.running_tasks,
            "completed_tasks": stats.completed_tasks,
            "failed_tasks": stats.failed_tasks,
            "active_agents": stats.active_agents
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Tick completed:");
        println!("  Active goals:    {}", stats.active_goals);
        println!("  Pending tasks:   {}", stats.pending_tasks);
        println!("  Running tasks:   {}", stats.running_tasks);
        println!("  Completed tasks: {}", stats.completed_tasks);
        println!("  Failed tasks:    {}", stats.failed_tasks);
        println!("  Active agents:   {}", stats.active_agents);
    }

    Ok(())
}

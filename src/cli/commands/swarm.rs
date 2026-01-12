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
    /// Start the swarm orchestrator (backgrounds the process)
    Start {
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

        /// Run in foreground (don't background)
        #[arg(long)]
        foreground: bool,

        /// Memory MCP server address (e.g., "http://localhost:9100")
        #[arg(long, env = "ABATHUR_MEMORY_SERVER")]
        memory_server: Option<String>,

        /// Tasks MCP server address (e.g., "http://localhost:9101")
        #[arg(long, env = "ABATHUR_TASKS_SERVER")]
        tasks_server: Option<String>,

        /// A2A gateway address (e.g., "http://localhost:8080")
        #[arg(long, env = "ABATHUR_A2A_GATEWAY")]
        a2a_gateway: Option<String>,

        /// Start MCP servers automatically (memory, tasks, a2a)
        #[arg(long)]
        with_mcp_servers: bool,
    },
    /// Stop the running swarm orchestrator
    Stop,
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
        SwarmCommand::Start {
            max_agents,
            poll_interval_ms,
            dry_run,
            max_goals,
            foreground,
            memory_server,
            tasks_server,
            a2a_gateway,
            with_mcp_servers,
        } => {
            start_swarm(
                max_agents,
                poll_interval_ms,
                dry_run,
                max_goals,
                foreground,
                json_mode,
                memory_server,
                tasks_server,
                a2a_gateway,
                with_mcp_servers,
            ).await
        }
        SwarmCommand::Stop => stop_swarm(json_mode).await,
        SwarmCommand::Status => show_status(json_mode).await,
        SwarmCommand::Active => show_active(json_mode).await,
        SwarmCommand::Config => show_config(json_mode).await,
        SwarmCommand::Tick => run_tick(json_mode).await,
    }
}

/// Path to the PID file
const PID_FILE: &str = ".abathur/swarm.pid";
/// Path to the log file for backgrounded swarm
const LOG_FILE: &str = ".abathur/swarm.log";

/// Check if a process with the given PID is running
fn is_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        // On Windows, use tasklist
        use std::process::Command;
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid)])
            .output();
        output
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

/// Read the PID from the PID file
fn read_pid_file() -> Option<u32> {
    std::fs::read_to_string(PID_FILE)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Write the PID to the PID file
fn write_pid_file(pid: u32) -> Result<()> {
    // Ensure .abathur directory exists
    std::fs::create_dir_all(".abathur")?;
    std::fs::write(PID_FILE, pid.to_string())?;
    Ok(())
}

/// Remove the PID file
fn remove_pid_file() -> Result<()> {
    if std::path::Path::new(PID_FILE).exists() {
        std::fs::remove_file(PID_FILE)?;
    }
    Ok(())
}

/// Check if swarm is already running
fn check_existing_swarm() -> Option<u32> {
    read_pid_file().and_then(|pid| {
        if is_process_running(pid) {
            Some(pid)
        } else {
            // Stale PID file, clean it up
            let _ = remove_pid_file();
            None
        }
    })
}

/// MCP server configuration passed to orchestrator
#[derive(Debug, Clone, Default)]
struct McpServerUrls {
    memory_server: Option<String>,
    tasks_server: Option<String>,
    a2a_gateway: Option<String>,
}

async fn start_swarm(
    max_agents: usize,
    poll_interval_ms: u64,
    dry_run: bool,
    _max_goals: Option<usize>,
    foreground: bool,
    json_mode: bool,
    memory_server: Option<String>,
    tasks_server: Option<String>,
    a2a_gateway: Option<String>,
    with_mcp_servers: bool,
) -> Result<()> {
    // Check if swarm is already running
    if let Some(pid) = check_existing_swarm() {
        if json_mode {
            let output = serde_json::json!({
                "error": true,
                "message": "Swarm is already running",
                "pid": pid
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("Swarm is already running (PID: {})", pid);
            println!("Use 'abathur swarm stop' to stop it first.");
        }
        return Ok(());
    }

    // Determine MCP server URLs - use provided or defaults if with_mcp_servers is set
    let mcp_urls = if with_mcp_servers {
        McpServerUrls {
            memory_server: memory_server.or_else(|| Some("http://127.0.0.1:9100".to_string())),
            tasks_server: tasks_server.or_else(|| Some("http://127.0.0.1:9101".to_string())),
            a2a_gateway: a2a_gateway.or_else(|| Some("http://127.0.0.1:8080".to_string())),
        }
    } else {
        McpServerUrls {
            memory_server,
            tasks_server,
            a2a_gateway,
        }
    };

    if foreground {
        // Run in foreground (original behavior)
        run_swarm_foreground(max_agents, poll_interval_ms, dry_run, json_mode, mcp_urls, with_mcp_servers).await
    } else {
        // Background the swarm
        start_swarm_background(max_agents, poll_interval_ms, dry_run, json_mode, mcp_urls, with_mcp_servers)
    }
}

fn start_swarm_background(
    max_agents: usize,
    poll_interval_ms: u64,
    dry_run: bool,
    json_mode: bool,
    mcp_urls: McpServerUrls,
    with_mcp_servers: bool,
) -> Result<()> {
    use std::process::{Command, Stdio};

    // Get the current executable path
    let exe = std::env::current_exe()?;

    // Build the command to run in foreground mode
    let mut cmd = Command::new(&exe);
    cmd.args(["swarm", "start", "--foreground"])
        .arg("--max-agents")
        .arg(max_agents.to_string())
        .arg("--poll-interval-ms")
        .arg(poll_interval_ms.to_string());

    if dry_run {
        cmd.arg("--dry-run");
    }

    // Pass MCP server URLs to background process
    if let Some(ref url) = mcp_urls.memory_server {
        cmd.arg("--memory-server").arg(url);
    }
    if let Some(ref url) = mcp_urls.tasks_server {
        cmd.arg("--tasks-server").arg(url);
    }
    if let Some(ref url) = mcp_urls.a2a_gateway {
        cmd.arg("--a2a-gateway").arg(url);
    }
    if with_mcp_servers {
        cmd.arg("--with-mcp-servers");
    }

    // Ensure .abathur directory exists for log file
    std::fs::create_dir_all(".abathur")?;

    // Open log file for stdout/stderr
    let log_file = std::fs::File::create(LOG_FILE)?;
    let log_file_err = log_file.try_clone()?;

    // Configure for background execution
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err));

    // On Unix, use setsid to detach from terminal
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                // Create a new session and process group
                libc::setsid();
                Ok(())
            });
        }
    }

    let child = cmd.spawn()?;
    let pid = child.id();

    // Write PID file
    write_pid_file(pid)?;

    if json_mode {
        let output = serde_json::json!({
            "status": "started",
            "pid": pid,
            "log_file": LOG_FILE,
            "pid_file": PID_FILE
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Swarm orchestrator started in background");
        println!("   PID: {}", pid);
        println!("   Log: {}", LOG_FILE);
        println!("   Max agents: {}", max_agents);
        println!("   Poll interval: {}ms", poll_interval_ms);
        if dry_run {
            println!("   Mode: DRY RUN (using mock substrate)");
        }
        println!();
        println!("Use 'abathur swarm status' to check status");
        println!("Use 'abathur swarm stop' to stop the swarm");
    }

    Ok(())
}

async fn stop_swarm(json_mode: bool) -> Result<()> {
    match check_existing_swarm() {
        Some(pid) => {
            // Kill the process
            #[cfg(unix)]
            {
                use std::process::Command;
                let status = Command::new("kill")
                    .arg(pid.to_string())
                    .status()?;

                if !status.success() {
                    // Try SIGKILL if SIGTERM didn't work
                    let _ = Command::new("kill")
                        .args(["-9", &pid.to_string()])
                        .status();
                }
            }
            #[cfg(not(unix))]
            {
                use std::process::Command;
                let _ = Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .status();
            }

            // Remove PID file
            remove_pid_file()?;

            if json_mode {
                let output = serde_json::json!({
                    "status": "stopped",
                    "pid": pid
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Swarm orchestrator stopped (PID: {})", pid);
            }
        }
        None => {
            if json_mode {
                let output = serde_json::json!({
                    "status": "not_running",
                    "message": "No swarm is currently running"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("No swarm is currently running");
            }
        }
    }

    Ok(())
}

async fn run_swarm_foreground(
    max_agents: usize,
    poll_interval_ms: u64,
    dry_run: bool,
    json_mode: bool,
    mcp_urls: McpServerUrls,
    with_mcp_servers: bool,
) -> Result<()> {
    use crate::adapters::sqlite::{
        create_pool, SqliteAgentRepository, SqliteGoalRepository, SqliteMemoryRepository, SqliteTaskRepository,
        SqliteWorktreeRepository, Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::SubstrateRegistry;
    use crate::domain::models::SubstrateType;
    use crate::services::McpServerConfig;
    use std::sync::Arc;

    // Write PID file for foreground mode too (so status works)
    write_pid_file(std::process::id())?;

    // Set up cleanup on exit
    let _cleanup = scopeguard::guard((), |_| {
        let _ = remove_pid_file();
    });

    // Initialize database
    let pool = create_pool("abathur.db", None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator.run_embedded_migrations(all_embedded_migrations()).await?;

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));
    let agent_repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));

    // Start MCP servers if requested
    let mcp_server_handles = if with_mcp_servers {
        if !json_mode {
            println!("Starting MCP servers...");
        }
        Some(start_mcp_servers(pool.clone(), &mcp_urls, json_mode).await?)
    } else {
        None
    };

    // Get substrate (use mock for dry-run)
    let registry = SubstrateRegistry::new();
    let substrate: Arc<dyn crate::domain::ports::Substrate> = if dry_run {
        Arc::from(registry.create_by_type(SubstrateType::Mock))
    } else {
        Arc::from(registry.default_substrate())
    };

    // Build MCP server configuration for agents
    let mcp_server_config = McpServerConfig {
        memory_server: mcp_urls.memory_server.clone(),
        tasks_server: mcp_urls.tasks_server.clone(),
        a2a_gateway: mcp_urls.a2a_gateway.clone(),
        auto_start_servers: with_mcp_servers,
        bind_host: "127.0.0.1".to_string(),
        base_port: 9100,
    };

    let config = SwarmConfig {
        max_agents,
        poll_interval_ms,
        mcp_servers: mcp_server_config,
        ..Default::default()
    };

    let orchestrator = SwarmOrchestrator::new(
        goal_repo,
        task_repo,
        worktree_repo,
        agent_repo,
        substrate,
        config.clone(),
    ).with_memory_repo(memory_repo);

    if !json_mode {
        println!("Starting Abathur Swarm Orchestrator");
        println!("   Max agents: {}", max_agents);
        println!("   Poll interval: {}ms", poll_interval_ms);
        if dry_run {
            println!("   Mode: DRY RUN (using mock substrate)");
        }
        if mcp_urls.memory_server.is_some() || mcp_urls.tasks_server.is_some() || mcp_urls.a2a_gateway.is_some() {
            println!("   MCP Servers:");
            if let Some(ref url) = mcp_urls.memory_server {
                println!("      Memory: {}", url);
            }
            if let Some(ref url) = mcp_urls.tasks_server {
                println!("      Tasks: {}", url);
            }
            if let Some(ref url) = mcp_urls.a2a_gateway {
                println!("      A2A Gateway: {}", url);
            }
        }
        println!();
    }

    // Run cold start analysis if memory is empty
    match orchestrator.cold_start().await {
        Ok(Some(report)) => {
            if !json_mode {
                println!("Cold start complete: {} memories created", report.memories_created);
                println!("   Project type: {}", report.project_type);
            }
        }
        Ok(None) => {
            if !json_mode {
                println!("Existing memories found, skipping cold start");
            }
        }
        Err(e) => {
            if !json_mode {
                println!("Warning: Cold start failed: {}", e);
            }
        }
    }

    // Start memory decay daemon for background maintenance
    if let Err(e) = orchestrator.start_decay_daemon().await {
        if !json_mode {
            println!("Warning: Failed to start decay daemon: {}", e);
        }
    } else if !json_mode {
        println!("Memory decay daemon started");
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
                SwarmEvent::GoalDecomposed { goal_id, task_count } => {
                    if !json_mode {
                        println!("  Goal decomposed into {} tasks ({})", task_count, goal_id);
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
                SwarmEvent::TaskSubmitted { task_id, task_title, goal_id } => {
                    if !json_mode {
                        println!("  Task submitted: {} ({}) for goal {}", task_title, task_id, goal_id);
                    }
                }
                SwarmEvent::TaskReady { task_id, task_title } => {
                    if !json_mode {
                        println!("  Task ready: {} ({})", task_title, task_id);
                    }
                }
                SwarmEvent::TaskSpawned { task_id, task_title, agent_type } => {
                    if !json_mode {
                        println!("  Task started: {} ({}) [agent: {:?}]", task_title, task_id, agent_type);
                    }
                }
                SwarmEvent::WorktreeCreated { task_id, path } => {
                    if !json_mode {
                        println!("  Worktree created: {} -> {}", task_id, path);
                    }
                }
                SwarmEvent::TaskCompleted { task_id, tokens_used } => {
                    if !json_mode {
                        println!("  Task completed: {} (tokens: {})", task_id, tokens_used);
                    }
                }
                SwarmEvent::TaskFailed { task_id, error, retry_count } => {
                    if !json_mode {
                        println!("  Task failed: {} - {} (attempt {})", task_id, error, retry_count);
                    }
                }
                SwarmEvent::TaskRetrying { task_id, attempt, max_attempts } => {
                    if !json_mode {
                        println!("  Task retrying: {} (attempt {}/{})", task_id, attempt, max_attempts);
                    }
                }
                SwarmEvent::TaskVerified { task_id, passed, checks_passed, checks_total } => {
                    if !json_mode {
                        let status = if *passed { "passed" } else { "failed" };
                        println!("  Task verified: {} - {} ({}/{})", task_id, status, checks_passed, checks_total);
                    }
                }
                SwarmEvent::TaskQueuedForMerge { task_id, stage } => {
                    if !json_mode {
                        println!("  Task queued for merge: {} (stage: {})", task_id, stage);
                    }
                }
                SwarmEvent::TaskMerged { task_id, commit_sha } => {
                    if !json_mode {
                        println!("  Task merged: {} (commit: {})", task_id, commit_sha);
                    }
                }
                SwarmEvent::EvolutionTriggered { template_name, trigger } => {
                    if !json_mode {
                        println!("  Evolution triggered: {} - {}", template_name, trigger);
                    }
                }
                SwarmEvent::SpecialistSpawned { specialist_type, trigger, task_id } => {
                    if !json_mode {
                        let task_info = task_id.map(|id| format!(" (task: {})", id)).unwrap_or_default();
                        println!("  Specialist spawned: {} - {}{}", specialist_type, trigger, task_info);
                    }
                }
                SwarmEvent::GoalAlignmentEvaluated { task_id, overall_score, passes } => {
                    if !json_mode {
                        let status = if *passes { "aligned" } else { "misaligned" };
                        println!("  Goal alignment: {} - {} ({:.0}%)", task_id, status, overall_score * 100.0);
                    }
                }
                SwarmEvent::RestructureTriggered { task_id, decision } => {
                    if !json_mode {
                        println!("  DAG restructure: {} - {}", task_id, decision);
                    }
                }
                SwarmEvent::SpawnLimitExceeded { parent_task_id, limit_type, current_value, limit_value } => {
                    if !json_mode {
                        println!("  Spawn limit exceeded: task {} - {} ({}/{})",
                            parent_task_id, limit_type, current_value, limit_value);
                    }
                }
                SwarmEvent::AgentCreated { agent_type, tier } => {
                    if !json_mode {
                        println!("  Agent created: {} (tier: {})", agent_type, tier);
                    }
                }
                SwarmEvent::StatusUpdate(stats) => {
                    if !json_mode && stats.active_goals > 0 {
                        println!("Status: {} active goals, {} running tasks, {} tokens used",
                            stats.active_goals, stats.running_tasks, stats.total_tokens_used);
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

    // Stop MCP servers if we started them
    if let Some(handles) = mcp_server_handles {
        if !json_mode {
            println!("Stopping MCP servers...");
        }
        stop_mcp_servers(handles);
    }

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

/// Handles for running MCP servers
struct McpServerHandles {
    memory_handle: Option<tokio::task::JoinHandle<()>>,
    tasks_handle: Option<tokio::task::JoinHandle<()>>,
    a2a_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Start MCP servers in background tasks
async fn start_mcp_servers(
    pool: sqlx::SqlitePool,
    urls: &McpServerUrls,
    json_mode: bool,
) -> Result<McpServerHandles> {
    use crate::adapters::mcp::{MemoryHttpServer, MemoryHttpConfig, TasksHttpServer, TasksHttpConfig, A2AHttpGateway, A2AHttpConfig};
    use crate::adapters::sqlite::{SqliteMemoryRepository, SqliteTaskRepository, SqliteGoalRepository};
    use crate::services::{MemoryService, TaskService};
    use std::sync::Arc;

    let mut handles = McpServerHandles {
        memory_handle: None,
        tasks_handle: None,
        a2a_handle: None,
    };

    // Start Memory HTTP server
    if let Some(ref url) = urls.memory_server {
        let port = extract_port(url).unwrap_or(9100);
        let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
        let memory_service = MemoryService::new(memory_repo);
        let config = MemoryHttpConfig {
            port,
            ..Default::default()
        };
        let server = MemoryHttpServer::new(memory_service, config);

        if !json_mode {
            println!("   Starting Memory server on port {}", port);
        }

        handles.memory_handle = Some(tokio::spawn(async move {
            if let Err(e) = server.serve().await {
                tracing::error!("Memory server error: {}", e);
            }
        }));
    }

    // Start Tasks HTTP server
    if let Some(ref url) = urls.tasks_server {
        let port = extract_port(url).unwrap_or(9101);
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
        let task_service = TaskService::new(task_repo, goal_repo);
        let config = TasksHttpConfig {
            port,
            ..Default::default()
        };
        let server = TasksHttpServer::new(task_service, config);

        if !json_mode {
            println!("   Starting Tasks server on port {}", port);
        }

        handles.tasks_handle = Some(tokio::spawn(async move {
            if let Err(e) = server.serve().await {
                tracing::error!("Tasks server error: {}", e);
            }
        }));
    }

    // Start A2A HTTP gateway
    if let Some(ref url) = urls.a2a_gateway {
        let port = extract_port(url).unwrap_or(8080);
        let config = A2AHttpConfig {
            port,
            ..Default::default()
        };
        let gateway = A2AHttpGateway::new(config);

        if !json_mode {
            println!("   Starting A2A gateway on port {}", port);
        }

        handles.a2a_handle = Some(tokio::spawn(async move {
            if let Err(e) = gateway.serve().await {
                tracing::error!("A2A gateway error: {}", e);
            }
        }));
    }

    // Give servers a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    Ok(handles)
}

/// Extract port from URL like "http://localhost:9100"
fn extract_port(url: &str) -> Option<u16> {
    url.split(':').last()?.parse().ok()
}

/// Stop MCP servers
fn stop_mcp_servers(handles: McpServerHandles) {
    if let Some(h) = handles.memory_handle {
        h.abort();
    }
    if let Some(h) = handles.tasks_handle {
        h.abort();
    }
    if let Some(h) = handles.a2a_handle {
        h.abort();
    }
}

async fn show_status(json_mode: bool) -> Result<()> {
    use crate::adapters::sqlite::{
        create_pool, SqliteGoalRepository, SqliteTaskRepository, SqliteWorktreeRepository,
    };
    use std::sync::Arc;
    use crate::domain::models::{GoalStatus, TaskStatus, WorktreeStatus};
    use crate::domain::ports::{GoalRepository, GoalFilter, TaskRepository, WorktreeRepository};

    // Check if swarm is running
    let (swarm_running, swarm_pid) = match check_existing_swarm() {
        Some(pid) => (true, Some(pid)),
        None => (false, None),
    };

    let pool = create_pool("abathur.db", None).await?;

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));

    // Get counts
    let active_goals = goal_repo.list(GoalFilter { status: Some(GoalStatus::Active), ..Default::default() }).await?.len();
    let pending_tasks = task_repo.list_by_status(TaskStatus::Pending).await?.len();
    let running_tasks = task_repo.list_by_status(TaskStatus::Running).await?.len();
    let active_worktrees = worktree_repo.list_by_status(WorktreeStatus::Active).await?.len();

    let status = if swarm_running { "running" } else { "stopped" };

    if json_mode {
        let mut output = serde_json::json!({
            "status": status,
            "active_goals": active_goals,
            "pending_tasks": pending_tasks,
            "running_tasks": running_tasks,
            "active_worktrees": active_worktrees
        });
        if let Some(pid) = swarm_pid {
            output["pid"] = serde_json::json!(pid);
        }
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Swarm Status");
        println!("============");
        if swarm_running {
            println!("Orchestrator:     RUNNING (PID: {})", swarm_pid.unwrap());
        } else {
            println!("Orchestrator:     STOPPED");
        }
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
        create_pool, SqliteAgentRepository, SqliteGoalRepository, SqliteMemoryRepository, SqliteTaskRepository,
        SqliteWorktreeRepository, Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::SubstrateRegistry;
    use std::sync::Arc;

    let pool = create_pool("abathur.db", None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator.run_embedded_migrations(all_embedded_migrations()).await?;

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));
    let agent_repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));

    let substrate: std::sync::Arc<dyn crate::domain::ports::Substrate> =
        std::sync::Arc::from(SubstrateRegistry::mock_substrate());

    let mut config = SwarmConfig::default();
    config.use_worktrees = false; // Disable worktrees for tick command

    let orchestrator = SwarmOrchestrator::new(
        goal_repo,
        task_repo,
        worktree_repo,
        agent_repo,
        substrate,
        config,
    ).with_memory_repo(memory_repo);

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

//! Swarm orchestration CLI commands.

use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Deserialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use std::collections::HashMap;
use std::sync::Arc;

use crate::adapters::sqlite::{
    SqliteAgentRepository, SqliteGoalRepository, SqliteMemoryRepository, SqliteTaskRepository,
    SqliteTrajectoryRepository, SqliteWorktreeRepository,
};
use crate::services::overseers::{
    BuildOverseer, CompilationOverseer, LintOverseer, OverseerClusterService,
    SecurityScanOverseer, TestSuiteOverseer, TypeCheckOverseer,
};
use crate::domain::models::goal_federation::{ContractSignal, ConvergenceContract};
use crate::domain::models::swarm_dag::{SwarmDag, SwarmDagNode, SwarmDagNodeState};
use crate::services::{SwarmConfig, SwarmOrchestrator, SwarmEvent};

type CliOrchestrator = SwarmOrchestrator<
    SqliteGoalRepository,
    SqliteTaskRepository,
    SqliteWorktreeRepository,
    SqliteAgentRepository,
    SqliteMemoryRepository,
>;

/// Build the default overseer cluster for a Rust project.
fn build_rust_overseer_cluster() -> OverseerClusterService {
    let mut cluster = OverseerClusterService::new();
    // Phase 1 (Cheap): compilation & type checking
    cluster.add(Box::new(CompilationOverseer::cargo_check()));
    cluster.add(Box::new(TypeCheckOverseer::cargo_check()));
    cluster.add(Box::new(BuildOverseer::cargo_build()));
    // Phase 2 (Moderate): lint & security
    cluster.add(Box::new(LintOverseer::cargo_clippy()));
    cluster.add(Box::new(SecurityScanOverseer::cargo_audit()));
    // Phase 3 (Expensive): test suite
    cluster.add(Box::new(TestSuiteOverseer::cargo_test()));
    cluster
}

/// Build an orchestrator with mock substrate for CLI commands.
async fn build_cli_orchestrator(config: SwarmConfig) -> Result<CliOrchestrator> {
    use crate::adapters::sqlite::{
        create_pool, Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::SubstrateRegistry;
    use crate::services::{EventBus, EventBusConfig, EventReactor, ReactorConfig, EventScheduler, SchedulerConfig};

    let pool = create_pool("sqlite:.abathur/abathur.db", None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator.run_embedded_migrations(all_embedded_migrations()).await?;

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));
    let agent_repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));

    let substrate: Arc<dyn crate::domain::ports::Substrate> =
        Arc::from(SubstrateRegistry::mock_substrate());

    let event_store: Arc<dyn crate::services::event_store::EventStore> =
        Arc::new(crate::adapters::sqlite::SqliteEventRepository::new(pool.clone(), crate::services::crypto::load_encryptor_from_env()));
    let event_bus = Arc::new(
        EventBus::new(EventBusConfig { persist_events: true, ..Default::default() })
            .with_store(event_store.clone()),
    );
    let event_reactor = Arc::new(
        EventReactor::new(event_bus.clone(), ReactorConfig::default())
            .with_store(event_store),
    );
    let event_scheduler = Arc::new(EventScheduler::new(event_bus.clone(), SchedulerConfig::default()).with_pool(pool.clone()));

    let trigger_rule_repo = Arc::new(
        crate::adapters::sqlite::SqliteTriggerRuleRepository::new(pool.clone()),
    );

    let trajectory_repo = Arc::new(SqliteTrajectoryRepository::new(pool.clone()));
    let overseer_cluster = Arc::new(build_rust_overseer_cluster());

    Ok(SwarmOrchestrator::new(
        goal_repo, task_repo, worktree_repo, agent_repo, substrate.clone(), config,
        event_bus, event_reactor, event_scheduler,
    )
    .with_memory_repo(memory_repo)
    .with_trigger_rule_repo(trigger_rule_repo)
    .with_intent_verifier(substrate)
    .with_trajectory_repo(trajectory_repo)
    .with_overseer_cluster(overseer_cluster)
    .with_pool(pool.clone()))
}

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

        /// Events server address (e.g., "http://localhost:9102")
        #[arg(long, env = "ABATHUR_EVENTS_SERVER")]
        events_server: Option<String>,

        /// Start MCP servers automatically (memory, tasks, a2a, events)
        #[arg(long)]
        with_mcp_servers: bool,

        /// Default execution mode: "convergent" (default), "direct", or "auto" (heuristic)
        #[arg(long, default_value = "convergent")]
        default_execution_mode: String,

        /// Default workflow for this swarm session
        #[arg(long)]
        workflow: Option<String>,

        /// Bypass permission checks for dangerous operations (e.g., auto-merge to main).
        /// Without this flag, the swarm will only create pull requests and never merge
        /// directly into the default branch.
        #[arg(long)]
        dangerously_skip_permissions: bool,

        /// Disable git worktree isolation for agents (overrides abathur.toml)
        #[arg(long)]
        no_worktrees: bool,

        /// Path to a DAG YAML specification file to execute on startup.
        /// When provided, the swarm parses the DAG, creates a parent goal,
        /// and delegates root nodes through the federation pipeline.
        #[arg(long)]
        dag: Option<String>,
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
    /// List pending human escalations
    Escalations,
    /// Respond to a human escalation
    Respond {
        /// Escalation event ID to respond to
        #[arg(long)]
        id: String,
        /// Decision: accept, reject, clarify, abort, defer
        #[arg(long)]
        decision: String,
        /// Optional message/clarification text
        #[arg(long)]
        message: Option<String>,
    },
    /// Connect to a cerebrate (child swarm) for federation
    Connect {
        /// URL of the cerebrate's federation endpoint
        url: String,
        /// Automatically connect on startup
        #[arg(long)]
        auto_connect: bool,
        /// Maximum concurrent delegations
        #[arg(long, default_value = "10")]
        max_concurrent: u32,
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
    /// Disconnect from a cerebrate
    Disconnect {
        /// ID of the cerebrate to disconnect
        id: String,
    },
    /// List connected cerebrates
    Cerebrates {
        /// Show details for a specific cerebrate
        id: Option<String>,
    },
    /// Show the federation tree (hive topology)
    Hive,
    /// Manage swarm DAGs (cross-swarm dependency pipelines)
    Dag {
        #[command(subcommand)]
        command: DagCommand,
    },
}

/// Subcommands for swarm DAG management.
#[derive(Subcommand, Debug)]
pub enum DagCommand {
    /// Validate a DAG YAML specification file (dry-run parse + cycle check)
    Create {
        /// Path to the YAML DAG specification file
        #[arg(long)]
        file: String,
    },
    /// Show status of all DAGs
    Status,
}

pub async fn execute(args: SwarmArgs, json_mode: bool) -> Result<()> {
    match args.command {
        SwarmCommand::Start {
            max_agents,
            dry_run,
            max_goals,
            foreground,
            memory_server,
            tasks_server,
            a2a_gateway,
            events_server,
            with_mcp_servers,
            default_execution_mode,
            workflow,
            dangerously_skip_permissions,
            no_worktrees,
            dag,
        } => {
            start_swarm(
                max_agents,
                dry_run,
                max_goals,
                foreground,
                json_mode,
                memory_server,
                tasks_server,
                a2a_gateway,
                events_server,
                with_mcp_servers,
                default_execution_mode,
                workflow,
                dangerously_skip_permissions,
                no_worktrees,
                dag,
            ).await
        }
        SwarmCommand::Stop => stop_swarm(json_mode).await,
        SwarmCommand::Status => show_status(json_mode).await,
        SwarmCommand::Active => show_active(json_mode).await,
        SwarmCommand::Config => show_config(json_mode).await,
        SwarmCommand::Tick => run_tick(json_mode).await,
        SwarmCommand::Escalations => show_escalations(json_mode).await,
        SwarmCommand::Respond { id, decision, message } => {
            respond_to_escalation(&id, &decision, message.as_deref(), json_mode).await
        }
        SwarmCommand::Connect { url, auto_connect, max_concurrent, yes } => {
            federation_connect(&url, auto_connect, max_concurrent, yes, json_mode).await
        }
        SwarmCommand::Disconnect { id } => {
            federation_disconnect(&id, json_mode).await
        }
        SwarmCommand::Cerebrates { id } => {
            federation_cerebrates(id.as_deref(), json_mode).await
        }
        SwarmCommand::Hive => {
            federation_hive(json_mode).await
        }
        SwarmCommand::Dag { command } => {
            match command {
                DagCommand::Create { file } => dag_create(&file, json_mode).await,
                DagCommand::Status => dag_status(json_mode).await,
            }
        }
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
    events_server: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn start_swarm(
    max_agents: usize,
    dry_run: bool,
    _max_goals: Option<usize>,
    foreground: bool,
    json_mode: bool,
    memory_server: Option<String>,
    tasks_server: Option<String>,
    a2a_gateway: Option<String>,
    events_server: Option<String>,
    with_mcp_servers: bool,
    default_execution_mode: String,
    workflow: Option<String>,
    dangerously_skip_permissions: bool,
    no_worktrees: bool,
    dag: Option<String>,
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
            events_server: events_server.or_else(|| Some("http://127.0.0.1:9102".to_string())),
        }
    } else {
        McpServerUrls {
            memory_server,
            tasks_server,
            a2a_gateway,
            events_server,
        }
    };

    if foreground {
        // Run in foreground (original behavior)
        run_swarm_foreground(max_agents, dry_run, json_mode, mcp_urls, with_mcp_servers, &default_execution_mode, workflow.as_deref(), dangerously_skip_permissions, no_worktrees, dag.as_deref()).await
    } else {
        // Background the swarm
        start_swarm_background(max_agents, dry_run, json_mode, mcp_urls, with_mcp_servers, &default_execution_mode, workflow.as_deref(), dangerously_skip_permissions, no_worktrees, dag.as_deref())
    }
}

#[allow(clippy::too_many_arguments)]
fn start_swarm_background(
    max_agents: usize,
    dry_run: bool,
    json_mode: bool,
    mcp_urls: McpServerUrls,
    with_mcp_servers: bool,
    default_execution_mode: &str,
    workflow: Option<&str>,
    dangerously_skip_permissions: bool,
    no_worktrees: bool,
    dag: Option<&str>,
) -> Result<()> {
    use std::process::{Command, Stdio};

    // Get the current executable path
    let exe = std::env::current_exe()?;

    // Build the command to run in foreground mode
    let mut cmd = Command::new(&exe);
    cmd.args(["swarm", "start", "--foreground"])
        .arg("--max-agents")
        .arg(max_agents.to_string());

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
    if let Some(ref url) = mcp_urls.events_server {
        cmd.arg("--events-server").arg(url);
    }
    if with_mcp_servers {
        cmd.arg("--with-mcp-servers");
    }
    cmd.arg("--default-execution-mode").arg(default_execution_mode);
    if let Some(wf) = workflow {
        cmd.arg("--workflow").arg(wf);
    }
    if dangerously_skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
    }
    if no_worktrees {
        cmd.arg("--no-worktrees");
    }
    if let Some(dag_file) = dag {
        cmd.arg("--dag").arg(dag_file);
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

    // Note: PID file is written by the child process in run_swarm_foreground()
    // Writing it here would cause the child to see its own PID and think the
    // swarm is already running, preventing it from starting.

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
        println!("   Architecture: event-driven");
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

#[allow(clippy::too_many_arguments)]
async fn run_swarm_foreground(
    max_agents: usize,
    dry_run: bool,
    json_mode: bool,
    mcp_urls: McpServerUrls,
    with_mcp_servers: bool,
    default_execution_mode: &str,
    workflow: Option<&str>,
    dangerously_skip_permissions: bool,
    no_worktrees: bool,
    dag_file: Option<&str>,
) -> Result<()> {
    use crate::adapters::sqlite::{
        create_pool, Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::SubstrateRegistry;
    use crate::domain::models::{ExecutionMode, SubstrateType};
    use crate::services::McpServerConfig;

    // Write PID file for foreground mode too (so status works)
    write_pid_file(std::process::id())?;

    // Set up cleanup on exit
    let _cleanup = scopeguard::guard((), |_| {
        let _ = remove_pid_file();
    });

    // Initialize database
    let pool = create_pool("sqlite:.abathur/abathur.db", None).await?;
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

    let execution_mode = match default_execution_mode.to_lowercase().as_str() {
        "convergent" => Some(ExecutionMode::Convergent { parallel_samples: None }),
        "direct" => Some(ExecutionMode::Direct),
        "auto" | "none" => None,
        other => {
            tracing::warn!("Unknown execution mode '{}', using convergent", other);
            Some(ExecutionMode::Convergent { parallel_samples: None })
        }
    };

    // Load application config (abathur.toml) for workflow and polling settings
    let app_config = match crate::services::config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to load abathur.toml, using defaults: {}", e);
            crate::services::config::Config::default()
        }
    };

    // Resolve the workflow template for this swarm. An explicit --workflow flag
    // wins; otherwise fall back to `config.default_workflow`. Both must resolve
    // against inline workflows or YAML files in `workflows_dir` — there are no
    // hardcoded fallbacks. A missing workflow is a hard error that points the
    // user at `abathur init` to scaffold defaults.
    let workflow_template = {
        let name = workflow.unwrap_or(app_config.default_workflow.as_str());
        let wf = app_config.resolve_workflow(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Workflow '{}' not found. Run `abathur init` to scaffold the default \
                 workflow YAMLs, or set `workflows_dir` in abathur.toml to point at a \
                 directory that contains them (currently: '{}').",
                name,
                app_config.workflows_dir,
            )
        })?;
        Some(wf)
    };

    // Collect all workflow templates so the Overmind can route tasks to the right spine.
    let all_workflows: Vec<_> = app_config
        .available_workflows()
        .into_iter()
        .filter_map(|(name, _, _, _)| app_config.resolve_workflow(&name))
        .collect();

    let config = SwarmConfig {
        max_agents,
        mcp_servers: mcp_server_config,
        default_execution_mode: execution_mode,
        workflow_template,
        all_workflows,
        dangerously_skip_permissions,
        polling: app_config.polling,
        use_worktrees: if no_worktrees { false } else { app_config.worktrees.enabled },
        overmind_max_turns: Some(app_config.overmind.max_turns),
        fetch_on_sync: app_config.worktrees.fetch_on_sync,
        ..Default::default()
    };

    tracing::info!(use_worktrees = config.use_worktrees, "Swarm config: worktrees");

    // Create shared EventBus with persistence for reactive event system
    let event_store: Arc<dyn crate::services::event_store::EventStore> =
        Arc::new(crate::adapters::sqlite::SqliteEventRepository::new(pool.clone(), crate::services::crypto::load_encryptor_from_env()));
    let event_bus = Arc::new(
        crate::services::EventBus::new(crate::services::EventBusConfig {
            persist_events: true,
            ..Default::default()
        })
        .with_store(event_store.clone()),
    );

    // Create EventReactor and EventScheduler.
    // Built-in handlers and schedules are registered by the orchestrator
    // in its run() method via register_builtin_handlers/register_builtin_schedules.
    let reactor = Arc::new(
        crate::services::EventReactor::new(event_bus.clone(), crate::services::ReactorConfig::default())
            .with_store(event_store),
    );

    let scheduler = Arc::new(
        crate::services::EventScheduler::new(
            event_bus.clone(),
            crate::services::SchedulerConfig::default(),
        )
        .with_pool(pool.clone()),
    );

    let trigger_rule_repo = Arc::new(
        crate::adapters::sqlite::SqliteTriggerRuleRepository::new(pool.clone()),
    );

    let trajectory_repo = Arc::new(SqliteTrajectoryRepository::new(pool.clone()));
    let overseer_cluster = Arc::new(build_rust_overseer_cluster());

    // Load adapters from .abathur/adapters/ and build registry
    let adapters_base = std::path::Path::new(".abathur");
    let loaded_adapters = crate::services::adapter_loader::load_adapters(adapters_base).await;
    let prompt_content = crate::services::adapter_loader::collect_prompt_content(&loaded_adapters);
    let adapter_registry = Arc::new(
        crate::services::adapter_registry::AdapterRegistry::from_loaded(
            loaded_adapters,
            prompt_content,
        ),
    );

    let orchestrator = SwarmOrchestrator::new(
        goal_repo,
        task_repo,
        worktree_repo,
        agent_repo,
        substrate.clone(),
        config.clone(),
        event_bus.clone(),
        reactor.clone(),
        scheduler,
    )
    .with_memory_repo(memory_repo)
    .with_trigger_rule_repo(trigger_rule_repo)
    .with_intent_verifier(substrate)
    .with_trajectory_repo(trajectory_repo)
    .with_overseer_cluster(overseer_cluster)
    .with_pool(pool.clone())
    .with_adapter_registry(adapter_registry);

    // Wire up budget-aware scheduling using thresholds from abathur.toml [budget] section
    let orchestrator = {
        let tracker_config = crate::services::budget_tracker::BudgetTrackerConfig::from_budget_config(&app_config.budget);
        let tracker = std::sync::Arc::new(
            crate::services::budget_tracker::BudgetTracker::new(tracker_config, event_bus.clone())
        );
        orchestrator.with_budget_tracker(tracker)
    };

    // Wire up quiet-window scheduling from [scheduling] + [[quiet_windows]] config
    let orchestrator = if app_config.scheduling.quiet_hours_enabled {
        let qw_repo = Arc::new(
            crate::adapters::sqlite::SqliteQuietWindowRepository::new(pool.clone()),
        );
        // Seed config-defined windows into the database (upsert by name)
        for wc in &app_config.quiet_windows {
            let tz = wc.timezone.as_deref()
                .unwrap_or(&app_config.scheduling.default_timezone);
            let desc = wc.description.as_deref().unwrap_or("");
            use crate::domain::ports::QuietWindowRepository;
            match qw_repo.get_by_name(&wc.name).await {
                Ok(Some(mut existing)) => {
                    existing.start_cron = wc.start_cron.clone();
                    existing.end_cron = wc.end_cron.clone();
                    existing.timezone = tz.to_string();
                    existing.description = desc.to_string();
                    if let Err(e) = qw_repo.update(&existing).await {
                        tracing::warn!(name = %wc.name, error = %e, "Failed to update quiet window from config");
                    }
                }
                Ok(None) => {
                    let window = crate::domain::models::quiet_window::QuietWindow::new(
                        &wc.name, desc, &wc.start_cron, &wc.end_cron, tz,
                    );
                    if let Err(e) = qw_repo.create(&window).await {
                        tracing::warn!(name = %wc.name, error = %e, "Failed to create quiet window from config");
                    }
                }
                Err(e) => {
                    tracing::warn!(name = %wc.name, error = %e, "Failed to check quiet window");
                }
            }
        }
        let cws = Arc::new(
            crate::services::cost_window_service::CostWindowService::new(qw_repo),
        );
        if let Err(e) = cws.reload_windows().await {
            tracing::warn!(error = %e, "Failed to load quiet windows into cache");
        }
        orchestrator.with_cost_window_service(cws)
    } else {
        orchestrator
    };

    // Wire federation if enabled in config.
    let fed_config = app_config.federation.clone();
    let orchestrator = if fed_config.enabled {
        let federation_service = Arc::new(
            crate::services::federation::FederationService::new(fed_config.clone(), event_bus.clone()),
        );

        // Register and auto-connect configured cerebrates.
        for cc in &fed_config.cerebrates {
            federation_service
                .register_cerebrate(&cc.id, &cc.display_name, &cc.url)
                .await;
            if cc.auto_connect
                && let Err(e) = federation_service.connect(&cc.id).await {
                    tracing::warn!(cerebrate_id = %cc.id, error = %e, "Federation auto-connect failed");
                }
        }

        // Start federation background loops (heartbeat, stall/orphan detection).
        federation_service.start().await;

        orchestrator.with_federation_service(federation_service)
    } else {
        orchestrator
    };

    if !json_mode {
        println!("Starting Abathur Swarm Orchestrator");
        println!("   Max agents: {}", max_agents);
        println!("   Architecture: event-driven");
        if dry_run {
            println!("   Mode: DRY RUN (using mock substrate)");
        }
        if dangerously_skip_permissions {
            println!("   Permissions: SKIPPED (dangerously-skip-permissions)");
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
        if fed_config.enabled {
            println!("   Federation: enabled");
            println!("      Role: {:?}", fed_config.role);
            println!("      Cerebrates: {}", fed_config.cerebrates.len());
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

    // Start outbox poller for reliable event delivery
    orchestrator.start_outbox_poller().await;

    // Start federation convergence daemons if federation is enabled.
    if fed_config.enabled {
        use crate::adapters::a2a::client::HttpA2AClient;
        use crate::adapters::sqlite::SqliteFederatedGoalRepository;

        let a2a_client: Arc<dyn crate::adapters::a2a::client::A2AClient> =
            Arc::new(HttpA2AClient::new_or_panic());
        let federated_goal_repo: Arc<dyn crate::domain::ports::FederatedGoalRepository> =
            Arc::new(SqliteFederatedGoalRepository::new(pool.clone()));

        // Overmind: start convergence poller to monitor child swarms.
        if let Err(e) = orchestrator
            .start_convergence_poller(a2a_client, federated_goal_repo)
            .await
            && !json_mode {
                println!("Warning: Failed to start convergence poller: {}", e);
            }

        // Cerebrate: start convergence publisher (no-op if role is Overmind).
        // The publisher needs the A2A gateway's in-memory task map. If an A2A
        // gateway is running in-process we would share its task map here.
        // For now, create a standalone task map — the A2A gateway integration
        // will unify these when running embedded.
        let a2a_tasks = Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        ));
        if let Err(e) = orchestrator.start_convergence_publisher(a2a_tasks).await
            && !json_mode {
                println!("Warning: Failed to start convergence publisher: {}", e);
            }
    }

    // If a DAG file was provided, parse it, create the parent goal, register
    // the DAG event handler on the reactor, and delegate root nodes so the
    // federation pipeline drives DAG progression alongside normal goal processing.
    if let Some(dag_path) = dag_file {
        use crate::domain::models::goal::GoalPriority;
        use crate::services::federation::{SwarmDagExecutor, SwarmDagEventHandler};
        use crate::services::goal_service::GoalService;

        let federation_service = orchestrator
            .federation_service()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!(
                "Federation must be enabled to run a DAG. Set [federation] enabled = true in abathur.toml"
            ))?;

        let mut dag = parse_dag_from_yaml(dag_path)?;

        let dag_goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
        let goal_service = GoalService::new(dag_goal_repo);
        let (goal, goal_events) = goal_service
            .create_goal(
                format!("DAG: {}", dag.name),
                format!("DAG pipeline: {}", dag.name),
                GoalPriority::Normal,
                None,
                vec![],
                vec![],
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create DAG parent goal: {}", e))?;

        for ev in goal_events {
            event_bus.publish(ev).await;
        }

        let executor = Arc::new(SwarmDagExecutor::new(federation_service, event_bus.clone()));

        let delegated = executor
            .start(&mut dag, &goal)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start DAG execution: {}", e))?;

        if !json_mode {
            println!("DAG '{}' started ({} root nodes delegated)", dag.name, delegated.len());
            print_dag_nodes(&dag);
            println!();
        }

        // Register the DAG event handler so convergence/failure events drive
        // the DAG forward automatically via the shared reactor.
        let swarm_dags = Arc::new(tokio::sync::RwLock::new({
            let mut map = std::collections::HashMap::new();
            map.insert(dag.id, dag);
            map
        }));
        let placeholder_goals = Arc::new(tokio::sync::RwLock::new({
            let mut map = std::collections::HashMap::new();
            map.insert(goal.id, goal);
            map
        }));

        let dag_handler = SwarmDagEventHandler::new(
            swarm_dags,
            executor,
            placeholder_goals,
        );
        reactor.register(Arc::new(dag_handler)).await;
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
                SwarmEvent::GoalIterationCompleted { goal_id, tasks_completed } => {
                    if !json_mode {
                        println!("Goal iteration completed: {} ({} tasks done)", goal_id, tasks_completed);
                    }
                }
                SwarmEvent::GoalPaused { goal_id, reason } => {
                    if !json_mode {
                        println!("Goal paused: {} - {}", goal_id, reason);
                    }
                }
                SwarmEvent::TaskSubmitted { task_id, task_title, goal_id } => {
                    if !json_mode {
                        match goal_id {
                            Some(gid) => println!("  Task created: {} ({}) for goal {}", task_title, task_id, gid),
                            None => println!("  Task created: {} ({})", task_title, task_id),
                        }
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
                SwarmEvent::TaskVerified { task_id, passed, checks_passed, checks_total, failures_summary } => {
                    if !json_mode {
                        let status = if *passed { "passed" } else { "failed" };
                        println!("  Task verified: {} - {} ({}/{})", task_id, status, checks_passed, checks_total);
                        if !passed
                            && let Some(summary) = failures_summary {
                                for line in summary.lines() {
                                    println!("    FAILED CHECK: {}", line);
                                }
                            }
                    }
                }
                SwarmEvent::TaskQueuedForMerge { task_id, stage } => {
                    if !json_mode {
                        println!("  Task queued for merge: {} (stage: {})", task_id, stage);
                    }
                }
                SwarmEvent::PullRequestCreated { task_id, pr_url, branch } => {
                    if !json_mode {
                        println!("  PR created: {} (branch: {}, url: {})", task_id, branch, pr_url);
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
                SwarmEvent::IntentVerificationStarted { goal_id, iteration } => {
                    if !json_mode {
                        println!("  Intent verification started: goal {} (iteration {})", goal_id, iteration);
                    }
                }
                SwarmEvent::IntentVerificationCompleted {
                    goal_id,
                    satisfaction,
                    confidence,
                    gaps_count,
                    iteration,
                    will_retry,
                } => {
                    if !json_mode {
                        let retry_status = if *will_retry { "will retry" } else { "final" };
                        println!(
                            "  Intent verification: goal {} - {} (confidence: {:.0}%, {} gaps, iteration {}) [{}]",
                            goal_id, satisfaction, confidence * 100.0, gaps_count, iteration, retry_status
                        );
                    }
                }
                SwarmEvent::ConvergenceCompleted {
                    goal_id,
                    converged,
                    iterations,
                    final_satisfaction,
                } => {
                    if !json_mode {
                        let status = if *converged { "CONVERGED" } else { "NOT CONVERGED" };
                        println!(
                            "  Convergence loop completed: goal {} - {} after {} iterations ({})",
                            goal_id, status, iterations, final_satisfaction
                        );
                    }
                }
                SwarmEvent::HumanEscalationRequired {
                    goal_id,
                    task_id,
                    reason,
                    urgency,
                    questions,
                    is_blocking,
                } => {
                    if !json_mode {
                        let blocking_str = if *is_blocking { " [BLOCKING]" } else { "" };
                        println!(
                            "  ⚠️  HUMAN ESCALATION REQUIRED{} ({}): {}",
                            blocking_str, urgency, reason
                        );
                        if let Some(gid) = goal_id {
                            println!("      Goal: {}", gid);
                        }
                        if let Some(tid) = task_id {
                            println!("      Task: {}", tid);
                        }
                        for q in questions {
                            println!("      ? {}", q);
                        }
                    }
                }
                SwarmEvent::HumanResponseReceived {
                    escalation_id,
                    decision,
                    allows_continuation,
                } => {
                    if !json_mode {
                        let cont_str = if *allows_continuation { "continuing" } else { "halted" };
                        println!(
                            "  Human response received for {}: {} - {}",
                            escalation_id, decision, cont_str
                        );
                    }
                }
                SwarmEvent::BranchVerificationStarted {
                    branch_task_ids,
                    waiting_task_ids,
                } => {
                    if !json_mode {
                        println!(
                            "  Branch verification started: {} branch tasks, {} waiting",
                            branch_task_ids.len(), waiting_task_ids.len()
                        );
                    }
                }
                SwarmEvent::BranchVerificationCompleted {
                    branch_satisfied,
                    dependents_can_proceed,
                    gaps_count,
                } => {
                    if !json_mode {
                        let status = if *branch_satisfied { "✓ satisfied" } else { "✗ not satisfied" };
                        let proceed = if *dependents_can_proceed { "proceeding" } else { "blocked" };
                        println!(
                            "  Branch verification completed: {} ({} gaps) - dependents {}",
                            status, gaps_count, proceed
                        );
                    }
                }
                SwarmEvent::SemanticDriftDetected {
                    goal_id,
                    recurring_gaps,
                    iterations,
                } => {
                    if !json_mode {
                        println!(
                            "  ⚠️  SEMANTIC DRIFT detected for goal {} after {} iterations",
                            goal_id, iterations
                        );
                        println!("      Recurring gaps that haven't been resolved:");
                        for gap in recurring_gaps {
                            println!("        - {}", gap);
                        }
                    }
                }
                SwarmEvent::TaskClaimed { task_id, agent_type } => {
                    if !json_mode {
                        println!("  Task claimed: {} by agent '{}'", task_id, agent_type);
                    }
                }
                SwarmEvent::AgentInstanceCompleted { instance_id, task_id, tokens_used } => {
                    if !json_mode {
                        println!("  Agent instance completed: {} for task {} ({} tokens)", instance_id, task_id, tokens_used);
                    }
                }
                SwarmEvent::ReconciliationCompleted { corrections_made } => {
                    if !json_mode && *corrections_made > 0 {
                        println!("  Reconciliation: {} corrections made", corrections_made);
                    }
                }
                SwarmEvent::SubtaskMergedToFeature { task_id, feature_branch } => {
                    if !json_mode {
                        println!("  Subtask merged to feature: {} → {}", task_id, feature_branch);
                    }
                }
                // Federation events
                SwarmEvent::FederationCerebrateConnected { cerebrate_id, capabilities } => {
                    if !json_mode {
                        println!("  🌐 Cerebrate connected: {} (capabilities: {})", cerebrate_id, capabilities.join(", "));
                    }
                }
                SwarmEvent::FederationCerebrateDisconnected { cerebrate_id, reason } => {
                    if !json_mode {
                        println!("  🌐 Cerebrate disconnected: {} ({})", cerebrate_id, reason);
                    }
                }
                SwarmEvent::FederationTaskDelegated { task_id, cerebrate_id } => {
                    if !json_mode {
                        println!("  🌐 Task delegated: {} → {}", task_id, cerebrate_id);
                    }
                }
                SwarmEvent::FederationTaskAccepted { task_id, cerebrate_id } => {
                    if !json_mode {
                        println!("  🌐 Task accepted: {} by {}", task_id, cerebrate_id);
                    }
                }
                SwarmEvent::FederationTaskRejected { task_id, cerebrate_id, reason } => {
                    if !json_mode {
                        println!("  🌐 Task rejected: {} by {} ({})", task_id, cerebrate_id, reason);
                    }
                }
                SwarmEvent::FederationProgressReceived { task_id, cerebrate_id, phase, progress_pct, summary } => {
                    if !json_mode {
                        println!("  🌐 Progress: {} from {} — {} ({:.0}%): {}", task_id, cerebrate_id, phase, progress_pct * 100.0, summary);
                    }
                }
                SwarmEvent::FederationResultReceived { task_id, cerebrate_id, status, summary, .. } => {
                    if !json_mode {
                        println!("  🌐 Result: {} from {} — {}: {}", task_id, cerebrate_id, status, summary);
                    }
                }
                SwarmEvent::FederationHeartbeatMissed { cerebrate_id, missed_count } => {
                    if !json_mode {
                        println!("  ⚠️ Heartbeat missed: {} (count: {})", cerebrate_id, missed_count);
                    }
                }
                SwarmEvent::FederationCerebrateUnreachable { cerebrate_id, in_flight_tasks } => {
                    if !json_mode {
                        println!("  ❌ Cerebrate unreachable: {} ({} in-flight tasks)", cerebrate_id, in_flight_tasks.len());
                    }
                }
                SwarmEvent::FederationStallDetected { task_id, cerebrate_id, stall_duration_secs } => {
                    if !json_mode {
                        println!("  ⚠️ Stall detected: task {} on {} ({}s with no progress)", task_id, cerebrate_id, stall_duration_secs);
                    }
                }
                SwarmEvent::FederationReactionEmitted { reaction_type, description, .. } => {
                    if !json_mode {
                        println!("  🌐 Reaction [{}]: {}", reaction_type, description);
                    }
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
    events_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Start MCP servers in background tasks
async fn start_mcp_servers(
    pool: sqlx::SqlitePool,
    urls: &McpServerUrls,
    json_mode: bool,
) -> Result<McpServerHandles> {
    use crate::adapters::mcp::{MemoryHttpServer, MemoryHttpConfig, TasksHttpServer, TasksHttpConfig, A2AHttpGateway, A2AHttpConfig, EventsHttpServer, EventsHttpConfig};
    use crate::adapters::sqlite::SqliteEventRepository;
    use crate::services::command_bus::CommandBus;
    use crate::services::{GoalService, MemoryService, TaskService, EventBus, EventBusConfig};

    let mut handles = McpServerHandles {
        memory_handle: None,
        tasks_handle: None,
        a2a_handle: None,
        events_handle: None,
    };

    // Create shared CommandBus for MCP servers
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
    let memory_service = MemoryService::new(memory_repo);
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let task_service = TaskService::new(task_repo);
    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let goal_service = GoalService::new(goal_repo);
    let mcp_event_bus = Arc::new(EventBus::new(EventBusConfig { persist_events: true, ..Default::default() }));
    let outbox_repo = Arc::new(crate::adapters::sqlite::SqliteOutboxRepository::new(pool.clone()));
    let command_bus = Arc::new(CommandBus::new(
        Arc::new(task_service.clone()),
        Arc::new(goal_service),
        Arc::new(memory_service.clone()),
        mcp_event_bus,
    ).with_pool(pool.clone()).with_outbox(outbox_repo));

    // Start Memory HTTP server
    if let Some(ref url) = urls.memory_server {
        let port = extract_port(url).unwrap_or(9100);
        let config = MemoryHttpConfig {
            port,
            ..Default::default()
        };
        let server = MemoryHttpServer::new(memory_service.clone(), command_bus.clone(), config);

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
        let config = TasksHttpConfig {
            port,
            ..Default::default()
        };
        let server = TasksHttpServer::new(task_service, command_bus, config);

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

    // Start Events HTTP server
    if let Some(ref url) = urls.events_server {
        let port = extract_port(url).unwrap_or(9102);
        let event_store = Arc::new(SqliteEventRepository::new(pool.clone(), crate::services::crypto::load_encryptor_from_env()));
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()).with_store(event_store.clone()));
        let config = EventsHttpConfig {
            port,
            ..Default::default()
        };
        let server = EventsHttpServer::new(event_bus, Some(event_store), config);

        if !json_mode {
            println!("   Starting Events server on port {}", port);
        }

        handles.events_handle = Some(tokio::spawn(async move {
            if let Err(e) = server.serve().await {
                tracing::error!("Events server error: {}", e);
            }
        }));
    }

    // Give servers a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    Ok(handles)
}

/// Extract port from URL like "http://localhost:9100"
fn extract_port(url: &str) -> Option<u16> {
    url.split(':').next_back()?.parse().ok()
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
    if let Some(h) = handles.events_handle {
        h.abort();
    }
}

async fn show_status(json_mode: bool) -> Result<()> {
    use crate::adapters::sqlite::create_pool;
    use crate::domain::models::{GoalStatus, TaskStatus, WorktreeStatus};
    use crate::domain::ports::{AgentFilter, AgentRepository, GoalRepository, GoalFilter, MemoryRepository, TaskRepository, WorktreeRepository};

    // Check if swarm is running
    let (swarm_running, swarm_pid) = match check_existing_swarm() {
        Some(pid) => (true, Some(pid)),
        None => (false, None),
    };

    let pool = create_pool("sqlite:.abathur/abathur.db", None).await?;

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
    let agent_repo = Arc::new(SqliteAgentRepository::new(pool.clone()));

    // Get counts
    let active_goals = goal_repo.list(GoalFilter { status: Some(GoalStatus::Active), ..Default::default() }).await?.len();
    let pending_tasks = task_repo.list_by_status(TaskStatus::Pending).await?.len();
    let running_tasks = task_repo.list_by_status(TaskStatus::Running).await?.len();
    let active_worktrees = worktree_repo.list_by_status(WorktreeStatus::Active).await?.len();

    // Task breakdown
    let ready_tasks = task_repo.list_by_status(TaskStatus::Ready).await?.len();
    let complete_tasks = task_repo.list_by_status(TaskStatus::Complete).await?.len();
    let failed_tasks = task_repo.list_by_status(TaskStatus::Failed).await?.len();

    // Memory health
    let memory_tier_counts = memory_repo.count_by_tier().await.unwrap_or_default();
    let working_memories = memory_tier_counts.get(&crate::domain::models::MemoryTier::Working).copied().unwrap_or(0);
    let episodic_memories = memory_tier_counts.get(&crate::domain::models::MemoryTier::Episodic).copied().unwrap_or(0);
    let semantic_memories = memory_tier_counts.get(&crate::domain::models::MemoryTier::Semantic).copied().unwrap_or(0);
    let total_memories = working_memories + episodic_memories + semantic_memories;

    // Agent templates
    let agent_templates = agent_repo.list_templates(AgentFilter::default()).await.map(|t| t.len()).unwrap_or(0);

    let status = if swarm_running { "running" } else { "stopped" };

    // Load federation config if available
    let federation_info = {
        let config_path = std::path::Path::new("abathur.toml");
        if config_path.exists() {
            if let Ok(contents) = tokio::fs::read_to_string(config_path).await {
                if let Ok(toml_val) = contents.parse::<toml::Value>() {
                    toml_val.get("federation").and_then(|f| {
                        let enabled = f.get("enabled")?.as_bool()?;
                        if !enabled {
                            return None;
                        }
                        let role = f.get("role").and_then(|r| r.as_str()).unwrap_or("unknown");
                        let cerebrates_count = f.get("cerebrates").and_then(|c| c.as_array()).map(|a| a.len()).unwrap_or(0);
                        Some((role.to_string(), cerebrates_count))
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    if json_mode {
        let mut output = serde_json::json!({
            "status": status,
            "active_goals": active_goals,
            "pending_tasks": pending_tasks,
            "running_tasks": running_tasks,
            "active_worktrees": active_worktrees,
            "tasks": {
                "ready": ready_tasks,
                "complete": complete_tasks,
                "failed": failed_tasks,
                "pending": pending_tasks,
                "running": running_tasks
            },
            "memory": {
                "working": working_memories,
                "episodic": episodic_memories,
                "semantic": semantic_memories,
                "total": total_memories
            },
            "agent_templates": agent_templates
        });
        if let Some(pid) = swarm_pid {
            output["pid"] = serde_json::json!(pid);
        }
        if let Some((ref role, cerebrate_count)) = federation_info {
            output["federation"] = serde_json::json!({
                "role": role,
                "cerebrates": cerebrate_count,
            });
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
        println!();
        println!("Task Breakdown");
        println!("--------------");
        println!("Ready:            {}", ready_tasks);
        println!("Complete:         {}", complete_tasks);
        println!("Failed:           {}", failed_tasks);
        println!();
        println!("Memory Health");
        println!("-------------");
        println!("Working:          {}", working_memories);
        println!("Episodic:         {}", episodic_memories);
        println!("Semantic:         {}", semantic_memories);
        println!("Total:            {}", total_memories);
        println!();
        println!("Agent Templates:  {}", agent_templates);
        if let Some((role, cerebrate_count)) = federation_info {
            println!("Federation:       {} ({} cerebrates)", role, cerebrate_count);
        }
    }

    Ok(())
}

async fn show_active(json_mode: bool) -> Result<()> {
    use crate::adapters::sqlite::create_pool;
    use crate::domain::models::{GoalStatus, TaskStatus};
    use crate::domain::ports::{GoalRepository, GoalFilter, TaskRepository};

    let pool = create_pool("sqlite:.abathur/abathur.db", None).await?;

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
            "goal_timeout_secs": config.goal_timeout_secs,
            "auto_retry": config.auto_retry,
            "max_task_retries": config.max_task_retries,
            "reconciliation_interval_secs": config.reconciliation_interval_secs
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Swarm Configuration");
        println!("===================");
        println!("Max agents:         {}", config.max_agents);
        println!("Default max turns:  {}", config.default_max_turns);
        println!("Use worktrees:      {}", config.use_worktrees);
        println!("Goal timeout (s):   {}", config.goal_timeout_secs);
        println!("Auto-retry:         {}", config.auto_retry);
        println!("Max task retries:   {}", config.max_task_retries);
        println!("Reconciliation (s): {:?}", config.reconciliation_interval_secs);
        println!("Skip permissions:   {}", config.dangerously_skip_permissions);
    }

    Ok(())
}

async fn show_escalations(json_mode: bool) -> Result<()> {
    let orchestrator = build_cli_orchestrator(SwarmConfig::default()).await?;

    let escalations = orchestrator.list_pending_escalations().await;

    if json_mode {
        let output: Vec<serde_json::Value> = escalations.iter().map(|e| {
            serde_json::json!({
                "id": e.id.to_string(),
                "goal_id": e.goal_id.map(|id| id.to_string()),
                "task_id": e.task_id.map(|id| id.to_string()),
                "reason": e.escalation.reason,
                "urgency": e.escalation.urgency.as_str(),
                "questions": e.escalation.questions,
                "is_blocking": e.is_blocking(),
                "created_at": e.created_at.to_rfc3339(),
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if escalations.is_empty() {
        println!("No pending escalations.");
    } else {
        println!("Pending Escalations ({}):", escalations.len());
        println!("{}", "=".repeat(60));
        for e in &escalations {
            println!("\nID:       {}", e.id);
            println!("Urgency:  {}", e.escalation.urgency.as_str());
            println!("Reason:   {}", e.escalation.reason);
            if let Some(gid) = e.goal_id {
                println!("Goal:     {}", gid);
            }
            if let Some(tid) = e.task_id {
                println!("Task:     {}", tid);
            }
            if !e.escalation.questions.is_empty() {
                println!("Questions:");
                for q in &e.escalation.questions {
                    println!("  - {}", q);
                }
            }
            println!("Blocking: {}", e.is_blocking());
            println!("Created:  {}", e.created_at.to_rfc3339());
        }
    }

    Ok(())
}

async fn respond_to_escalation(id: &str, decision: &str, message: Option<&str>, json_mode: bool) -> Result<()> {
    use crate::adapters::sqlite::create_pool;
    use crate::cli::id_resolver::resolve_event_id;
    use crate::domain::models::{EscalationDecision, HumanEscalationResponse};

    let pool = create_pool("sqlite:.abathur/abathur.db", None).await?;
    let event_id = resolve_event_id(&pool, id).await?;

    let escalation_decision = match decision {
        "accept" => EscalationDecision::Accept,
        "reject" => EscalationDecision::Reject,
        "abort" => EscalationDecision::Abort,
        "clarify" => {
            let clarification = message.unwrap_or("").to_string();
            EscalationDecision::Clarify { clarification }
        }
        "defer" => EscalationDecision::Defer { revisit_after: None },
        other => return Err(anyhow::anyhow!(
            "Unknown decision '{}'. Valid options: accept, reject, clarify, abort, defer", other
        )),
    };

    let response = HumanEscalationResponse {
        event_id,
        decision: escalation_decision,
        response_text: message.map(|m| m.to_string()),
        additional_context: None,
        responded_at: chrono::Utc::now(),
    };

    let orchestrator = build_cli_orchestrator(SwarmConfig::default()).await?;

    match orchestrator.respond_to_escalation(response, None).await {
        Ok(()) => {
            if json_mode {
                println!("{}", serde_json::json!({
                    "status": "ok",
                    "escalation_id": id,
                    "decision": decision,
                }));
            } else {
                println!("Response recorded for escalation {}.", id);
            }
        }
        Err(e) => {
            if json_mode {
                println!("{}", serde_json::json!({
                    "status": "error",
                    "error": e.to_string(),
                }));
            } else {
                eprintln!("Error: {}", e);
            }
        }
    }

    Ok(())
}

async fn run_tick(json_mode: bool) -> Result<()> {
    let config = SwarmConfig {
        use_worktrees: false, // Disable worktrees for tick command
        ..SwarmConfig::default()
    };

    let orchestrator = build_cli_orchestrator(config).await?;

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

// ============================================================================
// Federation CLI Commands
// ============================================================================

async fn federation_connect(
    url: &str,
    auto_connect: bool,
    max_concurrent: u32,
    yes: bool,
    json_mode: bool,
) -> Result<()> {
    use crate::services::federation::FederationService;
    use crate::services::event_bus::{EventBus, EventBusConfig};

    let config = crate::services::config::Config::load().unwrap_or_default();
    let fed_config = config.federation;

    if !fed_config.enabled {
        if json_mode {
            println!("{}", serde_json::json!({"error": "Federation is not enabled in abathur.toml"}));
        } else {
            println!("Federation is not enabled. Add [federation] enabled = true to abathur.toml");
        }
        return Ok(());
    }

    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let svc = FederationService::new(fed_config, event_bus);

    // Generate an ID from the URL
    let cerebrate_id = url
        .replace("https://", "")
        .replace("http://", "")
        .replace(':', "-")
        .replace('/', "");

    svc.register_cerebrate(&cerebrate_id, &cerebrate_id, url).await;

    if !yes && !json_mode {
        println!("Connecting to cerebrate at {}...", url);
        println!("  ID: {}", cerebrate_id);
        println!("  Max concurrent: {}", max_concurrent);
        println!("  Auto-connect: {}", auto_connect);
    }

    match svc.connect(&cerebrate_id).await {
        Ok(()) => {
            if json_mode {
                println!("{}", serde_json::json!({
                    "status": "connected",
                    "cerebrate_id": cerebrate_id,
                    "url": url,
                }));
            } else {
                println!("Connected to cerebrate: {}", cerebrate_id);
            }
        }
        Err(e) => {
            if json_mode {
                println!("{}", serde_json::json!({"error": e}));
            } else {
                println!("Failed to connect: {}", e);
            }
        }
    }

    Ok(())
}

async fn federation_disconnect(id: &str, json_mode: bool) -> Result<()> {
    use crate::services::federation::FederationService;
    use crate::services::event_bus::{EventBus, EventBusConfig};

    let config = crate::services::config::Config::load().unwrap_or_default();
    let fed_config = config.federation;

    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let svc = FederationService::new(fed_config, event_bus);

    // Load existing connections
    let base_path = std::path::Path::new(".abathur");
    let _ = svc.load_connections(base_path).await;

    match svc.disconnect(id).await {
        Ok(()) => {
            let _ = svc.save_connections(base_path).await;
            if json_mode {
                println!("{}", serde_json::json!({"status": "disconnected", "cerebrate_id": id}));
            } else {
                println!("Disconnected from cerebrate: {}", id);
            }
        }
        Err(e) => {
            if json_mode {
                println!("{}", serde_json::json!({"error": e}));
            } else {
                println!("Failed to disconnect: {}", e);
            }
        }
    }

    Ok(())
}

async fn federation_cerebrates(id: Option<&str>, json_mode: bool) -> Result<()> {
    use crate::services::federation::FederationService;
    use crate::services::event_bus::{EventBus, EventBusConfig};

    let config = crate::services::config::Config::load().unwrap_or_default();
    let fed_config = config.federation;

    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let svc = FederationService::new(fed_config, event_bus);

    // Load connections
    let base_path = std::path::Path::new(".abathur");
    let _ = svc.load_connections(base_path).await;

    if let Some(cerebrate_id) = id {
        // Show detail for one cerebrate
        match svc.get_cerebrate(cerebrate_id).await {
            Some(status) => {
                if json_mode {
                    println!("{}", serde_json::to_string_pretty(&status)?);
                } else {
                    println!("Cerebrate: {}", status.id);
                    println!("  Display name:  {}", status.display_name);
                    println!("  State:         {}", status.connection_state);
                    println!("  Load:          {:.1}%", status.load * 100.0);
                    println!("  Delegations:   {}/{}", status.active_delegations, status.max_concurrent_delegations);
                    println!("  Capabilities:  {}", if status.capabilities.is_empty() { "(none)".to_string() } else { status.capabilities.join(", ") });
                    if let Some(url) = &status.url {
                        println!("  URL:           {}", url);
                    }
                    if let Some(hb) = status.last_heartbeat_at {
                        println!("  Last heartbeat: {}", hb);
                    }
                }
            }
            None => {
                if json_mode {
                    println!("{}", serde_json::json!({"error": format!("Cerebrate not found: {}", cerebrate_id)}));
                } else {
                    println!("Cerebrate not found: {}", cerebrate_id);
                }
            }
        }
    } else {
        // List all cerebrates
        let cerebrates = svc.list_cerebrates().await;
        if cerebrates.is_empty() {
            if json_mode {
                println!("[]");
            } else {
                println!("No cerebrates registered.");
            }
            return Ok(());
        }

        if json_mode {
            println!("{}", serde_json::to_string_pretty(&cerebrates)?);
        } else {
            println!("{:<20} {:<15} {:<8} {:<12} CAPABILITIES", "ID", "STATE", "LOAD", "ACTIVE/MAX");
            println!("{}", "-".repeat(70));
            for c in &cerebrates {
                println!(
                    "{:<20} {:<15} {:<8.1}% {}/{:<10} {}",
                    c.id,
                    c.connection_state.to_string(),
                    c.load * 100.0,
                    c.active_delegations,
                    c.max_concurrent_delegations,
                    if c.capabilities.is_empty() { "-".to_string() } else { c.capabilities.join(", ") }
                );
            }
        }
    }

    Ok(())
}

async fn federation_hive(json_mode: bool) -> Result<()> {
    use crate::services::federation::FederationService;
    use crate::services::event_bus::{EventBus, EventBusConfig};

    let config = crate::services::config::Config::load().unwrap_or_default();
    let fed_config = config.federation.clone();

    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let svc = FederationService::new(fed_config.clone(), event_bus);

    // Load connections
    let base_path = std::path::Path::new(".abathur");
    let _ = svc.load_connections(base_path).await;

    let cerebrates = svc.list_cerebrates().await;

    if json_mode {
        let tree = serde_json::json!({
            "swarm_id": fed_config.swarm_id,
            "display_name": fed_config.display_name,
            "role": fed_config.role.to_string(),
            "cerebrates": cerebrates,
        });
        println!("{}", serde_json::to_string_pretty(&tree)?);
    } else {
        println!("Federation Hive:");
        println!("  {} ({}) [{}]", fed_config.display_name, fed_config.swarm_id, fed_config.role);
        if cerebrates.is_empty() {
            println!("    (no cerebrates)");
        } else {
            for (i, c) in cerebrates.iter().enumerate() {
                let prefix = if i == cerebrates.len() - 1 { "└─" } else { "├─" };
                let state_icon = match c.connection_state {
                    crate::domain::models::a2a::ConnectionState::Connected => "●",
                    crate::domain::models::a2a::ConnectionState::Disconnected => "○",
                    crate::domain::models::a2a::ConnectionState::Unreachable => "✗",
                    _ => "◐",
                };
                println!("    {} {} {} [{}] ({}/{})", prefix, state_icon, c.display_name, c.connection_state, c.active_delegations, c.max_concurrent_delegations);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// DAG YAML spec types
// ---------------------------------------------------------------------------

/// Intermediate representation for deserializing a signal from YAML.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SignalYamlSpec {
    BuildPassing,
    TestsPassing {
        #[serde(default = "default_min_pass_rate")]
        min_pass_rate: f64,
    },
    ConvergenceLevel {
        #[serde(default = "default_min_level")]
        min_level: f64,
    },
    TaskCompletionThreshold {
        min_completed: u32,
    },
    Custom {
        name: String,
        predicate: String,
    },
}

fn default_min_pass_rate() -> f64 {
    1.0
}

fn default_min_level() -> f64 {
    0.8 // Match the 80% threshold used by SwarmOverseer
}

impl SignalYamlSpec {
    fn into_contract_signal(self) -> ContractSignal {
        match self {
            Self::BuildPassing => ContractSignal::BuildPassing,
            Self::TestsPassing { min_pass_rate } => {
                ContractSignal::TestsPassing { min_pass_rate }
            }
            Self::ConvergenceLevel { min_level } => {
                ContractSignal::ConvergenceLevel { min_level }
            }
            Self::TaskCompletionThreshold { min_completed } => {
                ContractSignal::TaskCompletionThreshold { min_completed }
            }
            Self::Custom { name, predicate } => {
                ContractSignal::Custom { name, predicate }
            }
        }
    }
}

/// YAML representation of a convergence contract.
#[derive(Debug, Deserialize)]
struct ContractYamlSpec {
    #[serde(default = "default_poll_interval_secs")]
    poll_interval_secs: u64,
    #[serde(default)]
    required_signals: Vec<SignalYamlSpec>,
}

fn default_poll_interval_secs() -> u64 {
    60
}

/// YAML representation of a single DAG node.
#[derive(Debug, Deserialize)]
struct DagNodeYamlSpec {
    label: String,
    cerebrate: String,
    intent: String,
    #[serde(default)]
    depends_on: Vec<String>,
    contract: ContractYamlSpec,
}

/// Top-level YAML DAG specification.
#[derive(Debug, Deserialize)]
struct DagYamlSpec {
    name: String,
    nodes: Vec<DagNodeYamlSpec>,
}

impl DagYamlSpec {
    /// Convert this YAML spec into a validated `SwarmDag`.
    fn into_swarm_dag(self) -> Result<SwarmDag> {
        let mut dag = SwarmDag::new(&self.name);

        // First pass: assign UUIDs to labels.
        let mut label_to_id: HashMap<String, Uuid> = HashMap::new();
        for node_spec in &self.nodes {
            let id = Uuid::new_v4();
            if label_to_id.contains_key(&node_spec.label) {
                anyhow::bail!("Duplicate node label: '{}'", node_spec.label);
            }
            label_to_id.insert(node_spec.label.clone(), id);
        }

        // Second pass: build nodes with resolved dependencies.
        for node_spec in self.nodes {
            let node_id = label_to_id[&node_spec.label];

            let mut dependencies = Vec::new();
            for dep_label in &node_spec.depends_on {
                let dep_id = label_to_id.get(dep_label).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Node '{}' depends on unknown label '{}'",
                        node_spec.label,
                        dep_label
                    )
                })?;
                dependencies.push(*dep_id);
            }

            let contract = ConvergenceContract {
                poll_interval_secs: node_spec.contract.poll_interval_secs,
                required_signals: node_spec
                    .contract
                    .required_signals
                    .into_iter()
                    .map(|s| s.into_contract_signal())
                    .collect(),
            };

            dag.add_node(SwarmDagNode {
                id: node_id,
                label: node_spec.label,
                cerebrate_id: node_spec.cerebrate,
                intent: node_spec.intent,
                contract,
                dependencies,
                federated_goal_id: None,
                state: SwarmDagNodeState::Waiting,
            });
        }

        // Validate the constructed DAG.
        dag.validate().map_err(|e| anyhow::anyhow!("DAG validation failed: {}", e))?;

        Ok(dag)
    }
}

// ---------------------------------------------------------------------------
// DAG CLI commands
// ---------------------------------------------------------------------------

/// Parse a YAML DAG specification file into a validated `SwarmDag`.
fn parse_dag_from_yaml(file_path: &str) -> Result<SwarmDag> {
    let yaml_content = std::fs::read_to_string(file_path)
        .map_err(|e| anyhow::anyhow!("Failed to read DAG file '{}': {}", file_path, e))?;
    let spec: DagYamlSpec = serde_yaml::from_str(&yaml_content)
        .map_err(|e| anyhow::anyhow!("Failed to parse DAG YAML: {}", e))?;
    spec.into_swarm_dag()
}

async fn dag_create(file_path: &str, json_mode: bool) -> Result<()> {
    let dag = parse_dag_from_yaml(file_path)?;

    if json_mode {
        let output = serde_json::json!({
            "status": "created",
            "dag_id": dag.id.to_string(),
            "name": dag.name,
            "node_count": dag.nodes.len(),
            "nodes": dag.nodes.iter().map(|n| {
                serde_json::json!({
                    "id": n.id.to_string(),
                    "label": n.label,
                    "cerebrate": n.cerebrate_id,
                    "state": n.state.as_str(),
                    "dependencies": n.dependencies.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("DAG '{}' created successfully", dag.name);
        println!("  ID: {}", dag.id);
        println!("  Nodes: {}", dag.nodes.len());
        println!();

        // Print a visual topology.
        for node in &dag.nodes {
            let deps: Vec<String> = node.dependencies.iter().filter_map(|dep_id| {
                dag.get_node(*dep_id).map(|d| d.label.clone())
            }).collect();

            let dep_str = if deps.is_empty() {
                "(root)".to_string()
            } else {
                format!("depends on: [{}]", deps.join(", "))
            };

            println!(
                "  [{}] {} -> {} {}",
                node.state.as_str(),
                node.label,
                node.cerebrate_id,
                dep_str
            );
        }

        println!();
        println!("DAG is validated and ready. Use 'abathur swarm start --dag <path>' to begin execution.");
    }

    Ok(())
}

async fn dag_status(_json_mode: bool) -> Result<()> {
    // Placeholder: actual DAG persistence comes in a later phase.
    if _json_mode {
        let output = serde_json::json!({
            "dags": [],
            "message": "No active DAGs"
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("No active DAGs");
        println!();
        println!("Create a DAG with: abathur swarm dag create --file <path.yaml>");
    }

    Ok(())
}

/// Print DAG node status table.
fn print_dag_nodes(dag: &crate::domain::models::swarm_dag::SwarmDag) {
    for node in &dag.nodes {
        let deps: Vec<String> = node.dependencies.iter().filter_map(|dep_id| {
            dag.get_node(*dep_id).map(|d| d.label.clone())
        }).collect();

        let dep_str = if deps.is_empty() {
            "(root)".to_string()
        } else {
            format!("depends on: [{}]", deps.join(", "))
        };

        let fed_str = node.federated_goal_id
            .map(|id| format!(" [fed:{}]", id))
            .unwrap_or_default();

        println!(
            "  [{}] {} -> {} {}{}",
            node.state.as_str(),
            node.label,
            node.cerebrate_id,
            dep_str,
            fed_str,
        );
    }
}


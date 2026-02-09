//! MCP (Model Context Protocol) server CLI commands.
//!
//! Provides commands to start HTTP servers that expose Abathur's
//! capabilities to Claude Code agents.

use anyhow::Result;
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::mcp::{
    A2AHttpConfig, A2AHttpGateway, AgentsHttpConfig, AgentsHttpServer, MemoryHttpConfig,
    MemoryHttpServer, TasksHttpConfig, TasksHttpServer,
};
use crate::adapters::sqlite::{
    all_embedded_migrations, create_pool, Migrator, SqliteAgentRepository, SqliteGoalRepository,
    SqliteMemoryRepository, SqliteTaskRepository,
};
use crate::domain::models::a2a::A2AAgentCard;
use crate::services::command_bus::CommandBus;
use crate::services::{AgentService, GoalService, MemoryService, TaskService};

#[derive(Args, Debug)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCommand,
}

#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// Start the Memory HTTP server
    MemoryHttp {
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on
        #[arg(long, default_value = "9100")]
        port: u16,

        /// Disable CORS
        #[arg(long)]
        no_cors: bool,
    },
    /// Start the Tasks HTTP server
    TasksHttp {
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on
        #[arg(long, default_value = "9101")]
        port: u16,

        /// Disable CORS
        #[arg(long)]
        no_cors: bool,
    },
    /// Start the Agents HTTP server
    AgentsHttp {
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on
        #[arg(long, default_value = "9102")]
        port: u16,

        /// Disable CORS
        #[arg(long)]
        no_cors: bool,
    },
    /// Start the A2A HTTP gateway
    A2aHttp {
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on
        #[arg(long, default_value = "8080")]
        port: u16,

        /// Disable CORS
        #[arg(long)]
        no_cors: bool,

        /// Disable streaming (SSE)
        #[arg(long)]
        no_streaming: bool,

        /// Disable push notifications
        #[arg(long)]
        no_push: bool,

        /// Heartbeat interval for SSE streams (milliseconds)
        #[arg(long, default_value = "30000")]
        heartbeat_ms: u64,

        /// Maximum stream duration (seconds)
        #[arg(long, default_value = "3600")]
        max_stream_secs: u64,
    },
    /// Start all MCP servers
    All {
        /// Memory HTTP server port
        #[arg(long, default_value = "9100")]
        memory_port: u16,

        /// Tasks HTTP server port
        #[arg(long, default_value = "9101")]
        tasks_port: u16,

        /// Agents HTTP server port
        #[arg(long, default_value = "9102")]
        agents_port: u16,

        /// A2A HTTP gateway port
        #[arg(long, default_value = "8080")]
        a2a_port: u16,

        /// Host to bind all servers to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
    /// Start the MCP stdio server (JSON-RPC 2.0 over stdin/stdout)
    Stdio {
        /// Path to the abathur database
        #[arg(long)]
        db_path: String,

        /// Task ID for parent context (subtasks auto-set parent_id)
        #[arg(long)]
        task_id: Option<String>,
    },
    /// Show MCP server status
    Status,
}

pub async fn execute(args: McpArgs, json_mode: bool) -> Result<()> {
    match args.command {
        McpCommand::MemoryHttp {
            host,
            port,
            no_cors,
        } => start_memory_http(host, port, !no_cors, json_mode).await,
        McpCommand::TasksHttp {
            host,
            port,
            no_cors,
        } => start_tasks_http(host, port, !no_cors, json_mode).await,
        McpCommand::AgentsHttp {
            host,
            port,
            no_cors,
        } => start_agents_http(host, port, !no_cors, json_mode).await,
        McpCommand::A2aHttp {
            host,
            port,
            no_cors,
            no_streaming,
            no_push,
            heartbeat_ms,
            max_stream_secs,
        } => {
            start_a2a_http(
                host,
                port,
                !no_cors,
                !no_streaming,
                !no_push,
                heartbeat_ms,
                max_stream_secs,
                json_mode,
            )
            .await
        }
        McpCommand::All {
            memory_port,
            tasks_port,
            agents_port,
            a2a_port,
            host,
        } => start_all(host, memory_port, tasks_port, agents_port, a2a_port, json_mode).await,
        McpCommand::Stdio { db_path, task_id } => start_stdio(db_path, task_id).await,
        McpCommand::Status => show_status(json_mode).await,
    }
}

async fn start_memory_http(host: String, port: u16, enable_cors: bool, json_mode: bool) -> Result<()> {
    // Initialize database
    let pool = create_pool("abathur.db", None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator
        .run_embedded_migrations(all_embedded_migrations())
        .await?;

    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));

    let memory_service = MemoryService::new(memory_repo);
    let task_service = TaskService::new(task_repo);
    let goal_service = GoalService::new(goal_repo);

    let event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool);
    let command_bus = Arc::new(CommandBus::new(
        Arc::new(task_service),
        Arc::new(goal_service),
        Arc::new(memory_service.clone()),
        event_bus,
    ));

    let config = MemoryHttpConfig {
        host: host.clone(),
        port,
        enable_cors,
    };

    if json_mode {
        let output = serde_json::json!({
            "server": "memory-http",
            "status": "starting",
            "host": host,
            "port": port,
            "cors": enable_cors
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Starting MCP Memory HTTP Server");
        println!("   Host: {}", host);
        println!("   Port: {}", port);
        println!("   CORS: {}", if enable_cors { "enabled" } else { "disabled" });
        println!();
    }

    let server = MemoryHttpServer::new(memory_service, command_bus, config);
    server.serve().await.map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}

async fn start_tasks_http(host: String, port: u16, enable_cors: bool, json_mode: bool) -> Result<()> {
    // Initialize database
    let pool = create_pool("abathur.db", None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator
        .run_embedded_migrations(all_embedded_migrations())
        .await?;

    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));

    let task_service = TaskService::new(task_repo);
    let goal_service = GoalService::new(goal_repo);
    let memory_service = MemoryService::new(memory_repo);

    let event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool);
    let command_bus = Arc::new(CommandBus::new(
        Arc::new(task_service.clone()),
        Arc::new(goal_service),
        Arc::new(memory_service),
        event_bus,
    ));

    let config = TasksHttpConfig {
        host: host.clone(),
        port,
        enable_cors,
    };

    if json_mode {
        let output = serde_json::json!({
            "server": "tasks-http",
            "status": "starting",
            "host": host,
            "port": port,
            "cors": enable_cors
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Starting MCP Tasks HTTP Server");
        println!("   Host: {}", host);
        println!("   Port: {}", port);
        println!("   CORS: {}", if enable_cors { "enabled" } else { "disabled" });
        println!();
    }

    let server = TasksHttpServer::new(task_service, command_bus, config);
    server.serve().await.map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}

async fn start_agents_http(host: String, port: u16, enable_cors: bool, json_mode: bool) -> Result<()> {
    // Initialize database
    let pool = create_pool("abathur.db", None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator
        .run_embedded_migrations(all_embedded_migrations())
        .await?;

    let agent_repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
    let event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool);
    let service = AgentService::new(agent_repo, event_bus);

    let config = AgentsHttpConfig {
        host: host.clone(),
        port,
        enable_cors,
    };

    if json_mode {
        let output = serde_json::json!({
            "server": "agents-http",
            "status": "starting",
            "host": host,
            "port": port,
            "cors": enable_cors
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Starting MCP Agents HTTP Server");
        println!("   Host: {}", host);
        println!("   Port: {}", port);
        println!("   CORS: {}", if enable_cors { "enabled" } else { "disabled" });
        println!();
    }

    let server = AgentsHttpServer::new(service, config);
    server.serve().await.map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn start_a2a_http(
    host: String,
    port: u16,
    enable_cors: bool,
    enable_streaming: bool,
    enable_push: bool,
    heartbeat_ms: u64,
    max_stream_secs: u64,
    json_mode: bool,
) -> Result<()> {
    let config = A2AHttpConfig {
        host: host.clone(),
        port,
        enable_cors,
        enable_streaming,
        enable_push_notifications: enable_push,
        heartbeat_interval_ms: heartbeat_ms,
        max_stream_duration_s: max_stream_secs,
    };

    if json_mode {
        let output = serde_json::json!({
            "server": "a2a-http",
            "status": "starting",
            "host": host,
            "port": port,
            "cors": enable_cors,
            "streaming": enable_streaming,
            "push_notifications": enable_push
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Starting A2A HTTP Gateway");
        println!("   Host: {}", host);
        println!("   Port: {}", port);
        println!("   CORS: {}", if enable_cors { "enabled" } else { "disabled" });
        println!(
            "   Streaming: {}",
            if enable_streaming { "enabled" } else { "disabled" }
        );
        println!(
            "   Push notifications: {}",
            if enable_push { "enabled" } else { "disabled" }
        );
        println!();
    }

    let gateway = A2AHttpGateway::new(config);

    // Register default agent cards
    register_default_agents(&gateway).await;

    gateway.serve().await.map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}

async fn start_all(
    host: String,
    memory_port: u16,
    tasks_port: u16,
    agents_port: u16,
    a2a_port: u16,
    json_mode: bool,
) -> Result<()> {
    // Initialize database
    let pool = create_pool("abathur.db", None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator
        .run_embedded_migrations(all_embedded_migrations())
        .await?;

    if json_mode {
        let output = serde_json::json!({
            "servers": ["memory-http", "tasks-http", "agents-http", "a2a-http"],
            "status": "starting",
            "endpoints": {
                "memory": format!("http://{}:{}", host, memory_port),
                "tasks": format!("http://{}:{}", host, tasks_port),
                "agents": format!("http://{}:{}", host, agents_port),
                "a2a": format!("http://{}:{}", host, a2a_port)
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Starting All MCP Servers");
        println!("========================");
        println!("   Memory HTTP: http://{}:{}", host, memory_port);
        println!("   Tasks HTTP:  http://{}:{}", host, tasks_port);
        println!("   Agents HTTP: http://{}:{}", host, agents_port);
        println!("   A2A HTTP:    http://{}:{}", host, a2a_port);
        println!();
    }

    // Create services with shared persistent EventBus
    let shared_event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool.clone());

    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
    let memory_service = MemoryService::new(memory_repo);

    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let task_service = TaskService::new(task_repo);

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let goal_service = GoalService::new(goal_repo);

    let agent_repo = Arc::new(SqliteAgentRepository::new(pool));
    let agent_service = AgentService::new(agent_repo, shared_event_bus.clone());

    // Create shared CommandBus for mutation routing
    let command_bus = Arc::new(CommandBus::new(
        Arc::new(task_service.clone()),
        Arc::new(goal_service),
        Arc::new(memory_service.clone()),
        shared_event_bus,
    ));

    // Create servers
    let memory_config = MemoryHttpConfig {
        host: host.clone(),
        port: memory_port,
        enable_cors: true,
    };
    let memory_server = MemoryHttpServer::new(memory_service, command_bus.clone(), memory_config);

    let tasks_config = TasksHttpConfig {
        host: host.clone(),
        port: tasks_port,
        enable_cors: true,
    };
    let tasks_server = TasksHttpServer::new(task_service, command_bus, tasks_config);

    let agents_config = AgentsHttpConfig {
        host: host.clone(),
        port: agents_port,
        enable_cors: true,
    };
    let agents_server = AgentsHttpServer::new(agent_service, agents_config);

    let a2a_config = A2AHttpConfig {
        host: host.clone(),
        port: a2a_port,
        enable_cors: true,
        enable_streaming: true,
        enable_push_notifications: true,
        heartbeat_interval_ms: 30000,
        max_stream_duration_s: 3600,
    };
    let a2a_gateway = A2AHttpGateway::new(a2a_config);
    register_default_agents(&a2a_gateway).await;

    // Create shutdown signal
    let (shutdown_tx, shutdown_rx1) = tokio::sync::broadcast::channel::<()>(1);
    let shutdown_rx2 = shutdown_tx.subscribe();
    let shutdown_rx3 = shutdown_tx.subscribe();
    let shutdown_rx4 = shutdown_tx.subscribe();

    // Spawn all servers
    let memory_handle = tokio::spawn(async move {
        memory_server
            .serve_with_shutdown(async move {
                let _ = shutdown_rx1.resubscribe().recv().await;
            })
            .await
    });

    let tasks_handle = tokio::spawn(async move {
        tasks_server
            .serve_with_shutdown(async move {
                let _ = shutdown_rx2.resubscribe().recv().await;
            })
            .await
    });

    let agents_handle = tokio::spawn(async move {
        agents_server
            .serve_with_shutdown(async move {
                let _ = shutdown_rx3.resubscribe().recv().await;
            })
            .await
    });

    let a2a_handle = tokio::spawn(async move {
        a2a_gateway
            .serve_with_shutdown(async move {
                let _ = shutdown_rx4.resubscribe().recv().await;
            })
            .await
    });

    // Wait for Ctrl+C
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            if !json_mode {
                println!("\nShutting down servers...");
            }
            let _ = shutdown_tx.send(());
        }
        res = memory_handle => {
            if let Err(e) = res {
                eprintln!("Memory server error: {}", e);
            }
        }
        res = tasks_handle => {
            if let Err(e) = res {
                eprintln!("Tasks server error: {}", e);
            }
        }
        res = agents_handle => {
            if let Err(e) = res {
                eprintln!("Agents server error: {}", e);
            }
        }
        res = a2a_handle => {
            if let Err(e) = res {
                eprintln!("A2A server error: {}", e);
            }
        }
    }

    Ok(())
}

async fn start_stdio(db_path: String, task_id: Option<String>) -> Result<()> {
    use crate::adapters::mcp::StdioServer;
    use crate::adapters::sqlite::SqliteGoalRepository;

    // Parse optional task ID
    let task_uuid = match task_id {
        Some(ref id) => Some(
            uuid::Uuid::parse_str(id)
                .map_err(|e| anyhow::anyhow!("Invalid task ID '{}': {}", id, e))?,
        ),
        None => None,
    };

    // Initialize database
    let pool = create_pool(&db_path, None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator
        .run_embedded_migrations(all_embedded_migrations())
        .await?;

    // Create repositories and services with shared persistent EventBus
    let shared_event_bus = crate::cli::event_helpers::create_persistent_event_bus(pool.clone());

    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let task_service = TaskService::new(task_repo);

    let agent_repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
    let agent_service = AgentService::new(agent_repo, shared_event_bus.clone());

    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
    let memory_service = MemoryService::new(memory_repo);

    let goal_repo = Arc::new(SqliteGoalRepository::new(pool));
    let goal_service = GoalService::new(goal_repo.clone());

    // Create CommandBus for mutation routing
    let command_bus = Arc::new(CommandBus::new(
        Arc::new(task_service.clone()),
        Arc::new(goal_service),
        Arc::new(memory_service.clone()),
        shared_event_bus,
    ));

    let server = StdioServer::new(task_service, agent_service, memory_service, goal_repo, command_bus, task_uuid);
    server.run().await?;

    Ok(())
}

async fn show_status(json_mode: bool) -> Result<()> {
    use tokio::net::TcpStream;

    // Default ports to check
    let servers = [
        ("memory-http", "127.0.0.1", 9100),
        ("tasks-http", "127.0.0.1", 9101),
        ("agents-http", "127.0.0.1", 9102),
        ("a2a-http", "127.0.0.1", 8080),
    ];

    let mut results = Vec::new();

    for (name, host, port) in &servers {
        let addr = format!("{}:{}", host, port);
        let running = TcpStream::connect(&addr).await.is_ok();
        results.push((*name, *host, *port, running));
    }

    if json_mode {
        let output = serde_json::json!({
            "servers": results.iter().map(|(name, host, port, running)| {
                serde_json::json!({
                    "name": name,
                    "host": host,
                    "port": port,
                    "running": running
                })
            }).collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("MCP Server Status");
        println!("=================");
        for (name, host, port, running) in &results {
            let status = if *running { "RUNNING" } else { "STOPPED" };
            println!("  {:<12} {}:{:<5} {}", name, host, port, status);
        }
    }

    Ok(())
}

/// Register default agent cards for the A2A gateway.
async fn register_default_agents(gateway: &A2AHttpGateway) {
    use crate::domain::models::a2a::MessageType;

    // Overmind - the sole pre-packaged agent
    let overmind = A2AAgentCard::new("abathur.overmind")
        .with_display_name("Overmind")
        .with_description("Agentic orchestrator that analyzes tasks, dynamically creates agents, and delegates work")
        .with_capability("agent-creation")
        .with_capability("task-delegation")
        .with_capability("task-decomposition")
        .with_capability("goal-decomposition")
        .with_capability("strategic-planning")
        .accepts_message_type(MessageType::DelegateTask)
        .accepts_message_type(MessageType::HandoffRequest);

    gateway
        .register_agents(vec![overmind])
        .await;
}

//! MCP (Model Context Protocol) server CLI commands.
//!
//! Provides commands to start HTTP servers that expose Abathur's
//! capabilities to Claude Code agents.

use anyhow::Result;
use clap::{Args, Subcommand};
use std::sync::Arc;

use crate::adapters::mcp::{
    A2AHttpConfig, A2AHttpGateway, MemoryHttpConfig, MemoryHttpServer, TasksHttpConfig,
    TasksHttpServer,
};
use crate::adapters::sqlite::{
    all_embedded_migrations, create_pool, Migrator, SqliteMemoryRepository,
    SqliteTaskRepository,
};
use crate::domain::models::a2a::A2AAgentCard;
use crate::services::{MemoryService, TaskService};

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

        /// A2A HTTP gateway port
        #[arg(long, default_value = "8080")]
        a2a_port: u16,

        /// Host to bind all servers to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
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
            a2a_port,
            host,
        } => start_all(host, memory_port, tasks_port, a2a_port, json_mode).await,
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

    let repo = Arc::new(SqliteMemoryRepository::new(pool));
    let service = MemoryService::new(repo);

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

    let server = MemoryHttpServer::new(service, config);
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

    let task_repo = Arc::new(SqliteTaskRepository::new(pool));
    let service = TaskService::new(task_repo);

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

    let server = TasksHttpServer::new(service, config);
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
            "servers": ["memory-http", "tasks-http", "a2a-http"],
            "status": "starting",
            "endpoints": {
                "memory": format!("http://{}:{}", host, memory_port),
                "tasks": format!("http://{}:{}", host, tasks_port),
                "a2a": format!("http://{}:{}", host, a2a_port)
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Starting All MCP Servers");
        println!("========================");
        println!("   Memory HTTP: http://{}:{}", host, memory_port);
        println!("   Tasks HTTP:  http://{}:{}", host, tasks_port);
        println!("   A2A HTTP:    http://{}:{}", host, a2a_port);
        println!();
    }

    // Create services
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
    let memory_service = MemoryService::new(memory_repo);

    let task_repo = Arc::new(SqliteTaskRepository::new(pool));
    let task_service = TaskService::new(task_repo);

    // Create servers
    let memory_config = MemoryHttpConfig {
        host: host.clone(),
        port: memory_port,
        enable_cors: true,
    };
    let memory_server = MemoryHttpServer::new(memory_service, memory_config);

    let tasks_config = TasksHttpConfig {
        host: host.clone(),
        port: tasks_port,
        enable_cors: true,
    };
    let tasks_server = TasksHttpServer::new(task_service, tasks_config);

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

    let a2a_handle = tokio::spawn(async move {
        a2a_gateway
            .serve_with_shutdown(async move {
                let _ = shutdown_rx3.resubscribe().recv().await;
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
        res = a2a_handle => {
            if let Err(e) = res {
                eprintln!("A2A server error: {}", e);
            }
        }
    }

    Ok(())
}

async fn show_status(json_mode: bool) -> Result<()> {
    use tokio::net::TcpStream;

    // Default ports to check
    let servers = [
        ("memory-http", "127.0.0.1", 9100),
        ("tasks-http", "127.0.0.1", 9101),
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

    // Meta-planner agent
    let meta_planner = A2AAgentCard::new("abathur.meta-planner")
        .with_display_name("Meta Planner")
        .with_description("Decomposes high-level goals into executable task DAGs")
        .with_capability("goal-decomposition")
        .with_capability("task-planning")
        .with_capability("agent-selection")
        .with_handoff_target("code-implementer")
        .with_handoff_target("test-writer")
        .accepts_message_type(MessageType::DelegateTask);

    // Code implementer agent
    let code_implementer = A2AAgentCard::new("abathur.code-implementer")
        .with_display_name("Code Implementer")
        .with_description("Writes production code following architectural decisions")
        .with_capability("coding")
        .with_capability("implementation")
        .with_capability("rust")
        .with_capability("typescript")
        .with_handoff_target("test-writer")
        .with_handoff_target("documentation-writer")
        .accepts_message_type(MessageType::DelegateTask)
        .accepts_message_type(MessageType::HandoffRequest);

    // Test writer agent
    let test_writer = A2AAgentCard::new("abathur.test-writer")
        .with_display_name("Test Writer")
        .with_description("Creates comprehensive test suites for code")
        .with_capability("testing")
        .with_capability("unit-tests")
        .with_capability("integration-tests")
        .with_handoff_target("code-implementer")
        .accepts_message_type(MessageType::DelegateTask)
        .accepts_message_type(MessageType::HandoffRequest);

    // Documentation writer agent
    let doc_writer = A2AAgentCard::new("abathur.documentation-writer")
        .with_display_name("Documentation Writer")
        .with_description("Produces clear, accurate documentation")
        .with_capability("documentation")
        .with_capability("technical-writing")
        .accepts_message_type(MessageType::DelegateTask);

    // Integration verifier agent
    let verifier = A2AAgentCard::new("abathur.integration-verifier")
        .with_display_name("Integration Verifier")
        .with_description("Validates task completion and goal alignment")
        .with_capability("verification")
        .with_capability("testing")
        .with_capability("linting")
        .accepts_message_type(MessageType::DelegateTask);

    gateway
        .register_agents(vec![
            meta_planner,
            code_implementer,
            test_writer,
            doc_writer,
            verifier,
        ])
        .await;
}

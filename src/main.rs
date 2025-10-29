//! Abathur CLI entry point

use abathur_cli::{
    cli::{
        commands::{init, mcp, memory, swarm, task},
        service::{MemoryServiceAdapter, TaskQueueServiceAdapter},
        Cli, Commands, McpCommands, MemoryCommands, MemoryType, SwarmCommands, TaskCommands,
    },
    infrastructure::{
        config::ConfigLoader,
        database::{
            connection::DatabaseConnection, memory_repo::MemoryRepositoryImpl,
            task_repo::TaskRepositoryImpl,
        },
    },
    services::{DependencyResolver, MemoryService as RealMemoryService, PriorityCalculator, TaskQueueService as RealTaskQueueService},
};
use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // For init command, handle separately without database
    if matches!(cli.command, Commands::Init { .. }) {
        if let Commands::Init { force, template_repo, skip_clone } = cli.command {
            init::handle_init(force, &template_repo, skip_clone, cli.json).await?;
        }
        return Ok(());
    }

    // For swarm daemon mode, handle separately (it manages its own database connection)
    if let Commands::Swarm(SwarmCommands::Start { __daemon: true, max_agents }) = cli.command {
        return swarm::handle_daemon(max_agents).await;
    }

    // For MCP server commands, handle separately (they don't need the service layer)
    if let Commands::Mcp(mcp_cmd) = cli.command {
        return match mcp_cmd {
            McpCommands::MemoryHttp { db_path, port } => mcp::handle_memory_http(db_path, port).await,
            McpCommands::TasksHttp { db_path, port } => mcp::handle_tasks_http(db_path, port).await,
        };
    }

    // Load configuration
    let config = ConfigLoader::load().context("Failed to load configuration")?;

    // Initialize database connection
    let database_url = format!("sqlite:{}", config.database.path);
    let db = DatabaseConnection::new(&database_url)
        .await
        .context("Failed to connect to database")?;

    // Run migrations
    db.migrate()
        .await
        .context("Failed to run database migrations")?;

    // Initialize repositories
    let task_repo = Arc::new(TaskRepositoryImpl::new(db.pool().clone()));
    let memory_repo = Arc::new(MemoryRepositoryImpl::new(db.pool().clone()));

    // Initialize services
    let dependency_resolver = DependencyResolver::new();
    let priority_calc = PriorityCalculator::new();
    let real_task_service = RealTaskQueueService::new(
        task_repo.clone(),
        dependency_resolver,
        priority_calc,
    );
    let task_service = TaskQueueServiceAdapter::new(real_task_service);

    // Initialize real memory service with repository and adapter
    let real_memory_service = RealMemoryService::new(memory_repo);
    let memory_service = MemoryServiceAdapter::new(real_memory_service);

    match cli.command {
        Commands::Init { .. } => {
            // Already handled above
            unreachable!("Init command should be handled before this point");
        }
        Commands::Mcp(_) => {
            // Already handled above
            unreachable!("MCP commands should be handled before this point");
        }
        Commands::Task(task_cmd) => match task_cmd {
            TaskCommands::Submit {
                description,
                agent_type,
                summary,
                priority,
                dependencies,
            } => {
                task::handle_submit(&task_service, description, agent_type, summary, priority, dependencies, cli.json)
                    .await?;
            }
            TaskCommands::List { status, limit } => {
                let status_filter = status.and_then(|s| s.parse().ok());
                task::handle_list(&task_service, status_filter, limit, cli.json).await?;
            }
            TaskCommands::Show { task_id } => {
                task::handle_show(&task_service, task_id, cli.json).await?;
            }
            TaskCommands::Update {
                task_ids,
                status,
                priority,
                agent_type,
                add_dependency,
                remove_dependency,
                retry,
                cancel,
            } => {
                task::handle_update(
                    &task_service,
                    task_ids,
                    status,
                    priority,
                    agent_type,
                    add_dependency,
                    remove_dependency,
                    retry,
                    cancel,
                    cli.json,
                )
                .await?;
            }
            TaskCommands::Status => {
                task::handle_status(&task_service, cli.json).await?;
            }
            TaskCommands::Resolve => {
                task::handle_resolve(&task_service, cli.json).await?;
            }
        },
        Commands::Memory(memory_cmd) => match memory_cmd {
            MemoryCommands::List {
                namespace,
                memory_type,
                limit,
            } => {
                let mem_type = if let Some(mt_str) = memory_type {
                    Some(mt_str.parse::<MemoryType>()?)
                } else {
                    None
                };
                memory::handle_list(&memory_service, namespace, mem_type, limit, cli.json).await?;
            }
            MemoryCommands::Show { namespace, key } => {
                memory::handle_show(&memory_service, namespace, key, cli.json).await?;
            }
            MemoryCommands::Count {
                namespace,
                memory_type,
            } => {
                let mem_type = if let Some(mt_str) = memory_type {
                    Some(mt_str.parse::<MemoryType>()?)
                } else {
                    None
                };
                memory::handle_count(&memory_service, namespace, mem_type, cli.json).await?;
            }
        },
        Commands::Swarm(swarm_cmd) => match swarm_cmd {
            SwarmCommands::Start { max_agents, __daemon: _ } => {
                // Daemon mode is handled at the top of main()
                // This branch only handles normal CLI mode
                swarm::handle_start(&task_service, max_agents, cli.json).await?;
            }
            SwarmCommands::Stop => {
                swarm::handle_stop(&task_service, cli.json).await?;
            }
            SwarmCommands::Status => {
                swarm::handle_status(&task_service, cli.json).await?;
            }
        },
    }

    Ok(())
}

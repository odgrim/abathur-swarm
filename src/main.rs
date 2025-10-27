//! Abathur CLI entry point

use abathur_cli::{
    cli::{
        commands::{init, memory, swarm, task},
        service::{MemoryService, TaskQueueService as MockTaskQueueService, TaskQueueServiceAdapter},
        Cli, Commands, MemoryCommands, MemoryType, SwarmCommands, TaskCommands,
    },
    infrastructure::{
        config::ConfigLoader,
        database::{
            connection::DatabaseConnection, memory_repo::MemoryRepositoryImpl,
            task_repo::TaskRepositoryImpl,
        },
    },
    services::{DependencyResolver, PriorityCalculator, TaskQueueService as RealTaskQueueService},
};
use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // For init command, handle separately without database
    if matches!(cli.command, Commands::Init { .. }) {
        if let Commands::Init { force } = cli.command {
            init::handle_init(force, cli.json).await?;
        }
        return Ok(());
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
    let _memory_repo = Arc::new(MemoryRepositoryImpl::new(db.pool().clone()));

    // Initialize services
    let dependency_resolver = DependencyResolver::new();
    let priority_calc = PriorityCalculator::new();
    let real_task_service = RealTaskQueueService::new(
        task_repo.clone(),
        dependency_resolver,
        priority_calc,
    );
    let task_service = TaskQueueServiceAdapter::new(real_task_service);
    let mock_task_service = MockTaskQueueService::new(); // For swarm commands (temporary)
    let memory_service = MemoryService::new();

    match cli.command {
        Commands::Init { .. } => {
            // Already handled above
            unreachable!("Init command should be handled before this point");
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
            MemoryCommands::Show {
                namespace,
                key,
                version,
            } => {
                memory::handle_show(&memory_service, namespace, key, version, cli.json).await?;
            }
            MemoryCommands::Versions { namespace, key } => {
                memory::handle_versions(&memory_service, namespace, key, cli.json).await?;
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
            SwarmCommands::Start { max_agents, __daemon } => {
                if __daemon {
                    // Run in daemon mode - this is the background process
                    swarm::handle_daemon(max_agents).await?;
                } else {
                    // Normal CLI mode - spawn background process
                    swarm::handle_start(&mock_task_service, max_agents, cli.json).await?;
                }
            }
            SwarmCommands::Stop => {
                swarm::handle_stop(&mock_task_service, cli.json).await?;
            }
            SwarmCommands::Status => {
                swarm::handle_status(&mock_task_service, cli.json).await?;
            }
        },
    }

    Ok(())
}

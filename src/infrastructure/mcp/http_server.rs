//! HTTP MCP server implementation
//!
//! Provides HTTP MCP servers for memory and task queue management.

use crate::infrastructure::database::{
    connection::DatabaseConnection, memory_repo::MemoryRepositoryImpl, task_repo::TaskRepositoryImpl,
};
use crate::infrastructure::mcp::handlers::{
    handle_memory_request, handle_tasks_request, MemoryAppState, TasksAppState,
};
use crate::services::{DependencyResolver, MemoryService, PriorityCalculator, TaskQueueService};
use anyhow::{Context, Result};
use axum::{routing::post, Router};
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Start the Memory HTTP MCP server
pub async fn start_memory_server(db_path: String, port: u16) -> Result<()> {
    init_tracing();

    info!("Starting Abathur Memory HTTP MCP server");
    info!("Database path: {}", db_path);
    info!("Port: {}", port);

    let database_url = format!("sqlite:{}", db_path);
    let db = DatabaseConnection::new(&database_url)
        .await
        .context("Failed to connect to database")?;

    db.migrate()
        .await
        .context("Failed to run database migrations")?;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new(db.pool().clone()));
    let memory_service = Arc::new(MemoryService::new(memory_repo));

    info!("Database initialized successfully");

    let state = MemoryAppState { memory_service };

    let app = Router::new()
        .route("/", post(handle_memory_request))
        .with_state(state);

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("HTTP MCP server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Start the Tasks HTTP MCP server
pub async fn start_tasks_server(db_path: String, port: u16) -> Result<()> {
    init_tracing();

    info!("Starting Abathur Task Queue HTTP MCP server");
    info!("Database path: {}", db_path);
    info!("Port: {}", port);

    let database_url = format!("sqlite:{}", db_path);
    let db = DatabaseConnection::new(&database_url)
        .await
        .context("Failed to connect to database")?;

    db.migrate()
        .await
        .context("Failed to run database migrations")?;

    let task_repo = Arc::new(TaskRepositoryImpl::new(db.pool().clone()));
    let dependency_resolver = DependencyResolver::new();
    let priority_calc = PriorityCalculator::new();
    let task_service = Arc::new(TaskQueueService::new(
        task_repo.clone(),
        dependency_resolver,
        priority_calc,
    ));

    info!("Database initialized successfully");

    let state = TasksAppState { task_service };

    let app = Router::new()
        .route("/", post(handle_tasks_request))
        .with_state(state);

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("HTTP MCP server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Initialize tracing for HTTP servers
fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}

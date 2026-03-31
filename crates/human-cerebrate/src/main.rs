//! Human Cerebrate: Federation proxy for human task delegation via ClickUp.

mod clickup;
mod config;
mod heartbeat;
mod parser;
mod poller;
mod server;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use sqlx::sqlite::SqliteConnectOptions;
use std::str::FromStr;
use tracing::info;

use abathur::services::federation::service::FederationHttpClient;

use crate::clickup::client::ClickUpClient;
use crate::server::AppState;

#[derive(Parser)]
#[command(name = "human-cerebrate", about = "Federation proxy for human task delegation via ClickUp")]
struct Cli {
    /// Path to config file
    #[arg(long, default_value = "human-cerebrate.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Load config
    let config_str = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("Failed to read config file: {}", cli.config.display()))?;
    let config: config::Config =
        toml::from_str(&config_str).context("Failed to parse config file")?;

    // Read ClickUp API token from env
    let api_token = std::env::var("CLICKUP_API_TOKEN")
        .map_err(|_| anyhow::anyhow!("CLICKUP_API_TOKEN environment variable is required"))?;

    info!(
        cerebrate_id = %config.identity.cerebrate_id,
        "Starting human cerebrate proxy"
    );

    // Initialize SQLite
    let db = sqlx::SqlitePool::connect_with(
        SqliteConnectOptions::from_str(&config.database.path)?
            .create_if_missing(true),
    )
    .await
    .context("Failed to connect to SQLite database")?;

    state::run_migrations(&db).await.context("Failed to run migrations")?;

    // Build shared state
    let clickup = ClickUpClient::new(api_token);
    let federation_client = FederationHttpClient::new();

    let state = Arc::new(AppState {
        config,
        db,
        clickup: Arc::new(clickup),
        federation_client,
    });

    // Shutdown signal
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    // Spawn background tasks
    let heartbeat_handle = tokio::spawn(heartbeat::run_heartbeat(
        state.clone(),
        shutdown_tx.subscribe(),
    ));
    let poller_handle = tokio::spawn(poller::run_poller(
        state.clone(),
        shutdown_tx.subscribe(),
    ));

    // Build router
    let app = server::build_router(state.clone());
    let addr = SocketAddr::new(
        state.config.server.bind_address.parse()
            .context("Invalid bind address")?,
        state.config.server.port,
    );

    info!(%addr, "Starting server");

    // TLS or plaintext
    let has_tls = state.config.tls.cert_path.is_some() && state.config.tls.key_path.is_some();

    if has_tls {
        let cert_path = state.config.tls.cert_path.as_ref().unwrap();
        let key_path = state.config.tls.key_path.as_ref().unwrap();

        let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path)
            .await
            .context("Failed to load TLS certificates")?;

        let handle = axum_server::Handle::new();
        let handle_clone = handle.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            info!("Shutdown signal received");
            let _ = shutdown_tx.send(());
            handle_clone.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
        });

        axum_server::bind_rustls(addr, tls_config)
            .handle(handle)
            .serve(app.into_make_service())
            .await
            .context("Server error")?;
    } else {
        let handle = axum_server::Handle::new();
        let handle_clone = handle.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            info!("Shutdown signal received");
            let _ = shutdown_tx.send(());
            handle_clone.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
        });

        axum_server::bind(addr)
            .handle(handle)
            .serve(app.into_make_service())
            .await
            .context("Server error")?;
    }

    // Wait for background tasks
    let _ = heartbeat_handle.await;
    let _ = poller_handle.await;

    info!("Human cerebrate proxy shut down");
    Ok(())
}

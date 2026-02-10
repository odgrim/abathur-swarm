//! Abathur CLI entry point.

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use abathur::cli::{Cli, Commands};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init(args) => abathur::cli::commands::init::execute(args, cli.json).await,
        Commands::Goal(args) => abathur::cli::commands::goal::execute(args, cli.json).await,
        Commands::Task(args) => abathur::cli::commands::task::execute(args, cli.json).await,
        Commands::Memory(args) => abathur::cli::commands::memory::execute(args, cli.json).await,
        Commands::Agent(args) => abathur::cli::commands::agent::execute(args, cli.json).await,
        Commands::Worktree(args) => abathur::cli::commands::worktree::execute(args, cli.json).await,
        Commands::Swarm(args) => abathur::cli::commands::swarm::execute(args, cli.json).await,
        Commands::Mcp(args) => abathur::cli::commands::mcp::execute(args, cli.json).await,
        Commands::Trigger(args) => abathur::cli::commands::trigger::execute(args, cli.json).await,
        Commands::Event(args) => abathur::cli::commands::event::execute(args, cli.json).await,
    };

    if let Err(err) = result {
        abathur::cli::handle_error(err, cli.json);
    }
}

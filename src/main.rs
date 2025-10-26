mod cli;

use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();

    if let Err(e) = cli::run(cli).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

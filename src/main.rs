use abathur::DatabaseConnection;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Create database connection
    let db = DatabaseConnection::new("sqlite:.abathur/abathur.db").await?;

    // Run migrations
    db.migrate().await?;

    println!("Abathur database initialized successfully!");

    // Close connection
    db.close().await;

    Ok(())
}

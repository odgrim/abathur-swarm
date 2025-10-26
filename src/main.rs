use abathur::logging::{info, LogConfig, LoggerImpl};
use anyhow::Result;

fn main() -> Result<()> {
    // Initialize logging
    let config = LogConfig::default();
    let _logger = LoggerImpl::init(&config)?;

    info!("Abathur started");

    Ok(())
}

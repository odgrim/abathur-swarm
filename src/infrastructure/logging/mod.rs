//! Logging infrastructure using tracing
//!
//! This module provides structured logging with:
//! - JSON and pretty-print output formats
//! - Secret scrubbing for API keys, passwords, and tokens
//! - Log rotation with retention policies
//! - Async non-blocking file writes
//! - Environment-based filtering (`RUST_LOG`)
//!
//! # Examples
//!
//! ```rust,no_run
//! use abathur::infrastructure::logging::{LoggerImpl, LogConfig};
//! use anyhow::Result;
//!
//! fn main() -> Result<()> {
//!     let config = LogConfig::default();
//!     let _logger = LoggerImpl::init(&config)?;
//!
//!     // Use tracing macros
//!     tracing::info!("Application started");
//!     Ok(())
//! }
//! ```

mod config;
mod logger;
mod secret_scrubbing;

pub use config::{LogConfig, LogFormat, RotationPolicy};
pub use logger::{debug, error, info, instrument, trace, warn, LoggerImpl};
pub use secret_scrubbing::SecretScrubbingLayer;

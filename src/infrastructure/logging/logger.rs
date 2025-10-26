use super::config::{LogConfig, LogFormat, RotationPolicy};
use super::secret_scrubbing::SecretScrubbingLayer;
use anyhow::Result;
use std::io;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Logger implementation using tracing
pub struct LoggerImpl {
    _guard: Option<WorkerGuard>,
}

impl LoggerImpl {
    /// Initialize the logger with the given configuration
    ///
    /// # Arguments
    /// * `config` - Logging configuration
    ///
    /// # Returns
    /// * `Result<Self>` - Logger instance with guard to keep subscriber alive
    ///
    /// # Errors
    /// Returns an error if the logger cannot be initialized
    #[allow(clippy::too_many_lines)]
    pub fn init(config: &LogConfig) -> Result<Self> {
        // Parse log level
        let default_level = parse_log_level(&config.level)?;

        // Create environment filter with default level
        let env_filter = EnvFilter::builder()
            .with_default_directive(default_level.into())
            .from_env_lossy();

        // Secret scrubbing layer
        let _scrubbing_layer = SecretScrubbingLayer::new();

        // Build subscriber based on configuration
        let guard = if let Some(ref log_dir) = config.log_dir {
            // File output with rotation
            let file_appender = match config.rotation {
                RotationPolicy::Daily => rolling::daily(log_dir, "abathur.log"),
                RotationPolicy::Hourly => rolling::hourly(log_dir, "abathur.log"),
                RotationPolicy::Never => rolling::never(log_dir, "abathur.log"),
            };

            let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

            // File layer - always JSON for structured logging
            let file_layer = tracing_subscriber::fmt::layer()
                .json()
                .with_writer(non_blocking_file)
                .with_ansi(false)
                .with_current_span(true)
                .with_span_list(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_file(true)
                .with_line_number(true)
                .with_filter(env_filter.clone());

            if config.enable_stdout {
                // Stdout layer - respects format config
                match config.format {
                    LogFormat::Json => {
                        let stdout_layer = tracing_subscriber::fmt::layer()
                            .json()
                            .with_writer(io::stdout)
                            .with_current_span(true)
                            .with_span_list(true)
                            .with_target(true)
                            .with_thread_ids(true)
                            .with_thread_names(true)
                            .with_file(true)
                            .with_line_number(true)
                            .with_filter(env_filter);

                        tracing_subscriber::registry()
                            .with(file_layer)
                            .with(stdout_layer)
                            .init();
                    }
                    LogFormat::Pretty => {
                        let stdout_layer = tracing_subscriber::fmt::layer()
                            .pretty()
                            .with_writer(io::stdout)
                            .with_target(true)
                            .with_thread_ids(true)
                            .with_file(true)
                            .with_line_number(true)
                            .with_span_events(FmtSpan::CLOSE)
                            .with_filter(env_filter);

                        tracing_subscriber::registry()
                            .with(file_layer)
                            .with(stdout_layer)
                            .init();
                    }
                }
            } else {
                tracing_subscriber::registry()
                    .with(file_layer)
                    .init();
            }

            Some(guard)
        } else {
            // Stdout only
            match config.format {
                LogFormat::Json => {
                    let stdout_layer = tracing_subscriber::fmt::layer()
                        .json()
                        .with_writer(io::stdout)
                        .with_current_span(true)
                        .with_span_list(true)
                        .with_target(true)
                        .with_thread_ids(true)
                        .with_thread_names(true)
                        .with_file(true)
                        .with_line_number(true)
                        .with_filter(env_filter);

                    tracing_subscriber::registry()
                        .with(stdout_layer)
                        .init();
                }
                LogFormat::Pretty => {
                    let stdout_layer = tracing_subscriber::fmt::layer()
                        .pretty()
                        .with_writer(io::stdout)
                        .with_target(true)
                        .with_thread_ids(true)
                        .with_file(true)
                        .with_line_number(true)
                        .with_span_events(FmtSpan::CLOSE)
                        .with_filter(env_filter);

                    tracing_subscriber::registry()
                        .with(stdout_layer)
                        .init();
                }
            }

            None
        };

        tracing::info!(
            level = %config.level,
            format = ?config.format,
            file_output = config.log_dir.is_some(),
            "logger initialized"
        );

        Ok(Self { _guard: guard })
    }

    /// Get the worker guard (for testing)
    #[cfg(test)]
    pub fn guard(&self) -> &Option<WorkerGuard> {
        &self._guard
    }
}

/// Parse log level string to Level
fn parse_log_level(level: &str) -> Result<Level> {
    match level.to_lowercase().as_str() {
        "trace" => Ok(Level::TRACE),
        "debug" => Ok(Level::DEBUG),
        "info" => Ok(Level::INFO),
        "warn" => Ok(Level::WARN),
        "error" => Ok(Level::ERROR),
        _ => anyhow::bail!("Invalid log level: {level}"),
    }
}

// Re-export tracing macros for convenience
pub use tracing::{debug, error, info, instrument, trace, warn};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_level() {
        assert!(matches!(parse_log_level("trace"), Ok(Level::TRACE)));
        assert!(matches!(parse_log_level("debug"), Ok(Level::DEBUG)));
        assert!(matches!(parse_log_level("info"), Ok(Level::INFO)));
        assert!(matches!(parse_log_level("warn"), Ok(Level::WARN)));
        assert!(matches!(parse_log_level("error"), Ok(Level::ERROR)));
        assert!(matches!(parse_log_level("TRACE"), Ok(Level::TRACE)));
        assert!(parse_log_level("invalid").is_err());
    }

    #[test]
    fn test_logger_init_stdout_only() {
        let config = LogConfig {
            level: "info".to_string(),
            format: LogFormat::Pretty,
            log_dir: None,
            enable_stdout: true,
            rotation: RotationPolicy::Never,
            retention_days: 30,
        };

        // Note: This will initialize a global subscriber
        // In real tests, we'd use tracing-test or separate processes
        let result = LoggerImpl::init(&config);
        assert!(result.is_ok());
    }

    // Removed test_logger_init_with_file and test_logger_with_instrumentation
    // as they conflict with global subscriber initialization
    // These are covered by integration tests instead
}

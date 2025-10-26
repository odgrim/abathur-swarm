use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// Log level enumeration for structured logging
///
/// Levels are ordered from most verbose (Trace) to most severe (Error).
/// This ordering allows filtering and comparison operations.
///
/// # Examples
///
/// ```
/// use abathur::domain::ports::Level;
///
/// assert!(Level::Error > Level::Info);
/// assert!(Level::Trace < Level::Debug);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Most verbose level - detailed trace information
    Trace,
    /// Debug information useful during development
    Debug,
    /// Informational messages about normal operations
    Info,
    /// Warning messages for potentially problematic situations
    Warn,
    /// Error messages for failure conditions
    Error,
}

impl Level {
    /// Returns the string representation of the log level
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::ports::Level;
    ///
    /// assert_eq!(Level::Info.as_str(), "INFO");
    /// assert_eq!(Level::Error.as_str(), "ERROR");
    /// ```
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// Port trait for structured logging operations following hexagonal architecture
///
/// This trait defines the interface for logging operations in the Abathur system.
/// Implementations (adapters) can use different logging backends such as:
/// - `tracing` with file rotation
/// - `slog` with multiple outputs
/// - Cloud logging services (`CloudWatch`, `Stackdriver`)
/// - Test loggers that capture output for assertions
///
/// # Design Rationale
///
/// The Logger port exists to:
/// - Decouple domain/application logic from logging infrastructure
/// - Enable structured logging with contextual fields
/// - Support multiple logging backends through dependency injection
/// - Facilitate testing by allowing mock/test logger implementations
/// - Provide async logging to avoid blocking application threads
///
/// # Hexagonal Architecture
///
/// In hexagonal architecture:
/// - This trait is a **port** defined in the domain layer
/// - Concrete implementations are **adapters** in the infrastructure layer
/// - Application and domain code depend only on this trait, not on concrete loggers
/// - The logging backend can be swapped without changing business logic
///
/// # Structured Logging
///
/// The `log` method accepts structured fields as a `HashMap<String, Value>`.
/// This enables:
/// - Consistent log formatting across the system
/// - Easy filtering and searching in log aggregation tools
/// - Context propagation (trace IDs, user IDs, task IDs)
/// - Machine-readable logs for automated analysis
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` for safe use across async tasks.
/// Most logging backends are thread-safe by design, but implementations
/// should verify this guarantee.
///
/// # Performance Considerations
///
/// Logging is async to prevent blocking application threads. However:
/// - Avoid excessive logging in hot paths (use Debug/Trace levels)
/// - Consider buffering for high-volume logging scenarios
/// - Structured fields have serialization overhead - use judiciously
///
/// # Examples
///
/// ```
/// use abathur::domain::ports::{Logger, Level};
/// use std::collections::HashMap;
/// use serde_json::json;
///
/// async fn process_task(logger: &dyn Logger, task_id: &str) -> Result<(), String> {
///     logger.info(&format!("Starting task {}", task_id)).await;
///
///     // Structured logging with context
///     let mut fields = HashMap::new();
///     fields.insert("task_id".to_string(), json!(task_id));
///     fields.insert("step".to_string(), json!("validation"));
///     logger.log(Level::Debug, "Validating task inputs", fields).await;
///
///     // Convenience methods for common levels
///     logger.warn("Task taking longer than expected").await;
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait Logger: Send + Sync {
    /// Log a message with a specific level and structured fields
    ///
    /// This is the primary logging method that supports structured logging.
    /// Use this when you need to attach contextual information to log entries.
    ///
    /// # Arguments
    ///
    /// * `level` - The severity level of the log message
    /// * `message` - The human-readable log message
    /// * `fields` - Structured key-value pairs providing additional context
    ///
    /// # Field Guidelines
    ///
    /// Common field conventions:
    /// - `task_id`: UUID of the current task
    /// - `session_id`: UUID of the current session
    /// - `agent_type`: Name of the agent performing the operation
    /// - `user_id`: Identifier of the user
    /// - `error`: Error details for error-level logs
    /// - `duration_ms`: Operation duration in milliseconds
    /// - `trace_id`: Distributed tracing identifier
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::ports::{Logger, Level};
    /// use std::collections::HashMap;
    /// use serde_json::json;
    ///
    /// async fn example(logger: &dyn Logger) {
    ///     let mut fields = HashMap::new();
    ///     fields.insert("task_id".to_string(), json!("123e4567-e89b-12d3-a456-426614174000"));
    ///     fields.insert("agent_type".to_string(), json!("rust-testing-specialist"));
    ///     fields.insert("duration_ms".to_string(), json!(1234));
    ///
    ///     logger.log(Level::Info, "Task completed successfully", fields).await;
    /// }
    /// ```
    async fn log(&self, level: Level, message: &str, fields: HashMap<String, Value>);

    /// Log a trace-level message
    ///
    /// Trace is the most verbose level, used for detailed debugging information
    /// that is typically only enabled in development or when diagnosing specific issues.
    ///
    /// # Arguments
    ///
    /// * `message` - The trace message
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::ports::Logger;
    ///
    /// async fn example(logger: &dyn Logger) {
    ///     logger.trace("Entering function process_request").await;
    ///     // ... function logic ...
    ///     logger.trace("Exiting function process_request").await;
    /// }
    /// ```
    async fn trace(&self, message: &str);

    /// Log a debug-level message
    ///
    /// Debug messages provide information useful during development and troubleshooting.
    /// These are typically disabled in production unless investigating an issue.
    ///
    /// # Arguments
    ///
    /// * `message` - The debug message
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::ports::Logger;
    ///
    /// async fn example(logger: &dyn Logger) {
    ///     logger.debug("Cache hit for key: user_preferences").await;
    /// }
    /// ```
    async fn debug(&self, message: &str);

    /// Log an info-level message
    ///
    /// Info messages describe normal application operations and significant events.
    /// This is the standard level for production logging of important events.
    ///
    /// # Arguments
    ///
    /// * `message` - The informational message
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::ports::Logger;
    ///
    /// async fn example(logger: &dyn Logger) {
    ///     logger.info("Agent started successfully").await;
    ///     logger.info("Task queue initialized with 42 pending tasks").await;
    /// }
    /// ```
    async fn info(&self, message: &str);

    /// Log a warning-level message
    ///
    /// Warnings indicate potentially problematic situations that don't prevent
    /// the application from functioning but should be investigated.
    ///
    /// # Arguments
    ///
    /// * `message` - The warning message
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::ports::Logger;
    ///
    /// async fn example(logger: &dyn Logger) {
    ///     logger.warn("Task queue depth exceeds threshold (1000 tasks)").await;
    ///     logger.warn("Retry attempt 3/5 for external API call").await;
    /// }
    /// ```
    async fn warn(&self, message: &str);

    /// Log an error-level message
    ///
    /// Errors indicate failure conditions that prevent normal operation.
    /// These should be monitored and typically trigger alerts in production.
    ///
    /// # Arguments
    ///
    /// * `message` - The error message
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::ports::Logger;
    ///
    /// async fn example(logger: &dyn Logger) {
    ///     logger.error("Failed to connect to database after 5 retries").await;
    ///     logger.error("Task execution failed: invalid input parameters").await;
    /// }
    /// ```
    async fn error(&self, message: &str);
}

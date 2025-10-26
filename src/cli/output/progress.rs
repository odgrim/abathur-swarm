//! Progress bar utilities using indicatif for terminal output
//!
//! This module provides progress bars, spinners, and multi-progress managers
//! for displaying operation progress in the CLI.
//!
//! # Features
//! - Single progress bars for operations with known total
//! - Spinners for indeterminate operations
//! - Multi-progress for concurrent task tracking
//! - ETA calculation
//! - Customizable styling and templates

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::time::Duration;

/// Style templates for different progress bar types
const PROGRESS_TEMPLATE: &str =
    "[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg} (ETA: {eta})";
const SPINNER_TEMPLATE: &str = "[{elapsed_precise}] {spinner:.green} {msg}";
const SIMPLE_PROGRESS_TEMPLATE: &str = "{bar:40.cyan/blue} {pos}/{len} {msg}";

/// Progress bar characters for visual effect
const PROGRESS_CHARS: &str = "█▓▒░ ";
const SPINNER_CHARS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

/// Create a standard progress bar with ETA calculation
///
/// # Arguments
/// * `total` - Total number of items to process
///
/// # Returns
/// A configured ProgressBar with default styling
///
/// # Example
/// ```
/// use abathur::cli::output::progress::create_progress_bar;
///
/// let pb = create_progress_bar(100);
/// for i in 0..100 {
///     pb.set_message(format!("Processing item {}", i));
///     // do work
///     pb.inc(1);
/// }
/// pb.finish_with_message("Complete");
/// ```
pub fn create_progress_bar(total: u64) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(PROGRESS_TEMPLATE)
            .expect("Invalid progress bar template")
            .progress_chars(PROGRESS_CHARS),
    );
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

/// Create a simple progress bar without ETA (for faster rendering)
///
/// # Arguments
/// * `total` - Total number of items to process
///
/// # Returns
/// A ProgressBar with minimal styling
pub fn create_simple_progress_bar(total: u64) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(SIMPLE_PROGRESS_TEMPLATE)
            .expect("Invalid progress bar template")
            .progress_chars(PROGRESS_CHARS),
    );
    pb
}

/// Create a spinner for indeterminate operations
///
/// # Returns
/// A configured ProgressBar acting as a spinner
///
/// # Example
/// ```
/// use abathur::cli::output::progress::create_spinner;
///
/// let spinner = create_spinner();
/// spinner.set_message("Loading...");
/// // do work
/// spinner.finish_with_message("Done");
/// ```
pub fn create_spinner() -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template(SPINNER_TEMPLATE)
            .expect("Invalid spinner template")
            .tick_chars(SPINNER_CHARS),
    );
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner
}

/// Create a spinner with a custom message
///
/// # Arguments
/// * `message` - Initial message to display
///
/// # Returns
/// A configured spinner with the message set
pub fn create_spinner_with_message(message: impl Into<String>) -> ProgressBar {
    let spinner = create_spinner();
    spinner.set_message(message.into());
    spinner
}

/// Extension trait for ProgressBar to add common utility methods
pub trait ProgressBarExt {
    /// Finish with a success message (green checkmark)
    fn finish_success(&self, message: impl Into<String>);

    /// Finish with an error message (red X)
    fn finish_error(&self, message: impl Into<String>);

    /// Finish with a warning message (yellow !)
    fn finish_warning(&self, message: impl Into<String>);

    /// Update progress and message in one call
    fn update(&self, position: u64, message: impl Into<String>);
}

impl ProgressBarExt for ProgressBar {
    fn finish_success(&self, message: impl Into<String>) {
        self.finish_with_message(format!("✓ {}", message.into()));
    }

    fn finish_error(&self, message: impl Into<String>) {
        self.finish_with_message(format!("✗ {}", message.into()));
    }

    fn finish_warning(&self, message: impl Into<String>) {
        self.finish_with_message(format!("! {}", message.into()));
    }

    fn update(&self, position: u64, message: impl Into<String>) {
        self.set_position(position);
        self.set_message(message.into());
    }
}

/// Multi-progress manager for concurrent operations
///
/// Manages multiple progress bars or spinners displayed simultaneously,
/// useful for tracking parallel task execution.
///
/// # Example
/// ```
/// use abathur::cli::output::progress::MultiProgressManager;
///
/// let manager = MultiProgressManager::new();
///
/// // Add progress bars for concurrent tasks
/// let pb1 = manager.add_progress_bar(100, "Task 1");
/// let pb2 = manager.add_progress_bar(200, "Task 2");
/// let spinner = manager.add_spinner("Background task");
///
/// // Update progress independently
/// pb1.inc(50);
/// pb2.inc(100);
///
/// pb1.finish_with_message("Task 1 complete");
/// pb2.finish_with_message("Task 2 complete");
/// spinner.finish_with_message("Background complete");
/// ```
pub struct MultiProgressManager {
    multi: MultiProgress,
}

impl MultiProgressManager {
    /// Create a new multi-progress manager
    pub fn new() -> Self {
        Self {
            multi: MultiProgress::new(),
        }
    }

    /// Create a multi-progress manager with hidden output (for testing)
    pub fn hidden() -> Self {
        let multi = MultiProgress::new();
        multi.set_draw_target(ProgressDrawTarget::hidden());
        Self { multi }
    }

    /// Add a progress bar to the manager
    ///
    /// # Arguments
    /// * `total` - Total items for this progress bar
    /// * `message` - Initial message
    ///
    /// # Returns
    /// A ProgressBar that will be displayed in the multi-progress view
    pub fn add_progress_bar(&self, total: u64, message: impl Into<String>) -> ProgressBar {
        let pb = self.multi.add(create_progress_bar(total));
        pb.set_message(message.into());
        pb
    }

    /// Add a simple progress bar without ETA
    pub fn add_simple_progress_bar(&self, total: u64, message: impl Into<String>) -> ProgressBar {
        let pb = self.multi.add(create_simple_progress_bar(total));
        pb.set_message(message.into());
        pb
    }

    /// Add a spinner to the manager
    ///
    /// # Arguments
    /// * `message` - Spinner message
    ///
    /// # Returns
    /// A spinner ProgressBar
    pub fn add_spinner(&self, message: impl Into<String>) -> ProgressBar {
        let spinner = self.multi.add(create_spinner());
        spinner.set_message(message.into());
        spinner
    }

    /// Add a progress bar with custom styling
    ///
    /// # Arguments
    /// * `total` - Total items
    /// * `message` - Initial message
    /// * `template` - Custom template string
    /// * `progress_chars` - Custom progress characters
    pub fn add_custom_progress_bar(
        &self,
        total: u64,
        message: impl Into<String>,
        template: &str,
        progress_chars: &str,
    ) -> ProgressBar {
        let pb = self.multi.add(ProgressBar::new(total));
        pb.set_style(
            ProgressStyle::default_bar()
                .template(template)
                .expect("Invalid progress bar template")
                .progress_chars(progress_chars),
        );
        pb.set_message(message.into());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    /// Get a reference to the underlying MultiProgress
    pub fn inner(&self) -> &MultiProgress {
        &self.multi
    }

    /// Clear all progress bars (useful for cleanup)
    pub fn clear(&self) {
        self.multi.clear().ok();
    }
}

impl Default for MultiProgressManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a progress bar for agent execution
///
/// Specialized progress bar for tracking agent task execution with
/// appropriate styling and messaging.
pub fn create_agent_progress_bar(agent_type: &str, task_count: u64) -> ProgressBar {
    let pb = create_progress_bar(task_count);
    pb.set_message(format!("Agent: {}", agent_type));
    pb
}

/// Create a multi-progress setup for concurrent agent execution
///
/// # Arguments
/// * `agents` - Slice of agent type names
///
/// # Returns
/// A MultiProgressManager with spinners for each agent
pub fn create_agent_multi_progress(agents: &[&str]) -> MultiProgressManager {
    let manager = MultiProgressManager::new();

    for agent in agents {
        manager.add_spinner(format!("Agent: {}", agent));
    }

    manager
}

/// Create a progress bar for database operations
///
/// Specialized styling for database migrations or batch operations.
pub fn create_database_progress_bar(operation: &str, total: u64) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.green/yellow} {pos}/{len} {msg}")
            .expect("Invalid progress bar template")
            .progress_chars("=>-"),
    );
    pb.set_message(operation.to_string());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_progress_bar() {
        let pb = create_progress_bar(100);
        assert_eq!(pb.length().unwrap(), 100);
        pb.finish();
    }

    #[test]
    fn test_create_simple_progress_bar() {
        let pb = create_simple_progress_bar(50);
        assert_eq!(pb.length().unwrap(), 50);
        pb.finish();
    }

    #[test]
    fn test_create_spinner() {
        let spinner = create_spinner();
        spinner.set_message("Testing");
        spinner.finish();
    }

    #[test]
    fn test_create_spinner_with_message() {
        let spinner = create_spinner_with_message("Initial message");
        spinner.finish();
    }

    #[test]
    fn test_progress_bar_ext_success() {
        let pb = create_progress_bar(10);
        pb.finish_success("Operation completed");
    }

    #[test]
    fn test_progress_bar_ext_error() {
        let pb = create_progress_bar(10);
        pb.finish_error("Operation failed");
    }

    #[test]
    fn test_progress_bar_ext_warning() {
        let pb = create_progress_bar(10);
        pb.finish_warning("Operation has warnings");
    }

    #[test]
    fn test_progress_bar_ext_update() {
        let pb = create_progress_bar(100);
        ProgressBarExt::update(&pb, 50, "Halfway done");
        assert_eq!(pb.position(), 50);
        pb.finish();
    }

    #[test]
    fn test_multi_progress_manager_new() {
        let _manager = MultiProgressManager::new();
        // Note: MultiProgress may default to hidden when no terminal is available (tests)
        // This is expected behavior - the manager is created successfully
    }

    #[test]
    fn test_multi_progress_manager_hidden() {
        let manager = MultiProgressManager::hidden();
        assert!(manager.inner().is_hidden());
    }

    #[test]
    fn test_multi_progress_add_progress_bar() {
        let manager = MultiProgressManager::hidden();
        let pb = manager.add_progress_bar(100, "Test task");
        assert_eq!(pb.length().unwrap(), 100);
        pb.finish();
    }

    #[test]
    fn test_multi_progress_add_simple_progress_bar() {
        let manager = MultiProgressManager::hidden();
        let pb = manager.add_simple_progress_bar(50, "Simple task");
        assert_eq!(pb.length().unwrap(), 50);
        pb.finish();
    }

    #[test]
    fn test_multi_progress_add_spinner() {
        let manager = MultiProgressManager::hidden();
        let spinner = manager.add_spinner("Loading");
        spinner.finish();
    }

    #[test]
    fn test_multi_progress_add_custom_progress_bar() {
        let manager = MultiProgressManager::hidden();
        let pb = manager.add_custom_progress_bar(100, "Custom task", "{bar:40} {pos}/{len}", "=>-");
        assert_eq!(pb.length().unwrap(), 100);
        pb.finish();
    }

    #[test]
    fn test_multi_progress_concurrent_bars() {
        let manager = MultiProgressManager::hidden();

        let pb1 = manager.add_progress_bar(100, "Task 1");
        let pb2 = manager.add_progress_bar(200, "Task 2");
        let spinner = manager.add_spinner("Background task");

        pb1.inc(50);
        pb2.inc(100);

        assert_eq!(pb1.position(), 50);
        assert_eq!(pb2.position(), 100);

        pb1.finish_success("Task 1 complete");
        pb2.finish_success("Task 2 complete");
        spinner.finish_success("Background complete");
    }

    #[test]
    fn test_multi_progress_clear() {
        let manager = MultiProgressManager::hidden();
        let _pb = manager.add_progress_bar(100, "Test");
        manager.clear();
    }

    #[test]
    fn test_create_agent_progress_bar() {
        let pb = create_agent_progress_bar("test-agent", 10);
        assert_eq!(pb.length().unwrap(), 10);
        pb.finish();
    }

    #[test]
    fn test_create_agent_multi_progress() {
        let agents = vec!["agent1", "agent2", "agent3"];
        let manager = create_agent_multi_progress(&agents);
        manager.clear();
    }

    #[test]
    fn test_create_database_progress_bar() {
        let pb = create_database_progress_bar("Migration", 5);
        assert_eq!(pb.length().unwrap(), 5);
        pb.finish();
    }

    #[test]
    fn test_progress_bar_increment() {
        let pb = create_progress_bar(100);
        pb.inc(10);
        assert_eq!(pb.position(), 10);
        pb.inc(20);
        assert_eq!(pb.position(), 30);
        pb.finish();
    }

    #[test]
    fn test_spinner_messages() {
        let spinner = create_spinner();
        spinner.set_message("Step 1");
        spinner.set_message("Step 2");
        spinner.set_message("Step 3");
        spinner.finish();
    }

    #[test]
    fn test_multi_progress_default() {
        let manager = MultiProgressManager::default();
        let pb = manager.add_progress_bar(10, "Default test");
        pb.finish();
    }
}

//! CLI interface module
//!
//! This module contains all command-line interface components including:
//! - Command definitions and handlers
//! - Terminal output formatting (tables, trees, progress bars)
//! - TUI (Terminal User Interface) components

pub mod output;

// Re-export commonly used items
pub use output::progress::{
    MultiProgressManager, ProgressBarExt, create_progress_bar, create_spinner,
};

<<<<<<< HEAD
//! Terminal output formatting utilities
//!
//! This module provides utilities for formatting CLI output including:
//! - Table rendering (using comfy-table)
//! - Tree visualization
//! - Progress bars and spinners (using indicatif)

pub mod progress;

// Re-export commonly used items
pub use progress::{MultiProgressManager, ProgressBarExt, create_progress_bar, create_spinner};
=======
pub mod table;
pub mod tree;
>>>>>>> task_cli-structure_20251025-210033

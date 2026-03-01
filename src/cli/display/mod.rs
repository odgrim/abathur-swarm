//! Display framework for CLI output formatting.
//!
//! Provides shared primitives for colors, tables, formatting, and detail views
//! used across all CLI command output.

pub mod colors;
pub mod detail;
pub mod format;
pub mod table;

use serde::Serialize;

pub use colors::*;
pub use detail::*;
pub use format::*;
pub use table::*;

/// Trait for types that can be rendered as human-readable or JSON output.
pub trait CommandOutput: Serialize {
    fn to_human(&self) -> String;
    fn to_json(&self) -> serde_json::Value;
}

/// Dispatch output based on JSON mode flag.
pub fn output<T: CommandOutput>(result: &T, json_mode: bool) {
    if json_mode {
        println!(
            "{}",
            serde_json::to_string_pretty(&result.to_json()).unwrap_or_default()
        );
    } else {
        println!("{}", result.to_human());
    }
}

/// Render a success action result.
pub fn action_success(message: &str) -> String {
    use colored::Colorize;
    format!("{} {}", "\u{2713}".green().bold(), message)
}

/// Render a failure action result.
pub fn action_failure(message: &str) -> String {
    use colored::Colorize;
    format!("{} {}", "\u{2717}".red().bold(), message)
}

/// Truncate a string to a maximum length, appending "..." if truncated.
///
/// Kept for backward compatibility with existing imports.
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

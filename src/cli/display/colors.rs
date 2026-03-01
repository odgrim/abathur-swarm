//! Status, priority, and tier color mapping for CLI output.
//!
//! All coloring respects `NO_COLOR` env var automatically via the `colored` crate.

use colored::Colorize;

/// Returns a colored string for any status value.
///
/// Color scheme:
/// - Green:  complete, active
/// - Yellow: running, validating
/// - Blue:   pending, ready
/// - Cyan:   blocked
/// - Red:    failed
/// - Dim:    canceled, retired, deprecated, disabled
/// - White:  unknown/default
pub fn colorize_status(status: &str) -> colored::ColoredString {
    match status.to_lowercase().as_str() {
        "complete" | "completed" | "active" => status.green().bold(),
        "running" | "validating" => status.yellow(),
        "pending" | "ready" => status.blue(),
        "paused" => status.yellow().dimmed(),
        "blocked" => status.cyan(),
        "failed" => status.red().bold(),
        "canceled" | "cancelled" | "retired" | "deprecated" | "disabled" => status.dimmed(),
        _ => status.white(),
    }
}

/// Returns a colored string for priority values.
///
/// Critical = red bold, High = red, Normal = white, Low = dim.
pub fn colorize_priority(priority: &str) -> colored::ColoredString {
    match priority.to_lowercase().as_str() {
        "critical" => priority.red().bold(),
        "high" => priority.red(),
        "normal" => priority.white(),
        "low" => priority.dimmed(),
        _ => priority.white(),
    }
}

/// Returns a colored string for agent tier values.
///
/// Architect = magenta bold, Specialist = cyan, Worker = white.
pub fn colorize_tier(tier: &str) -> colored::ColoredString {
    match tier.to_lowercase().as_str() {
        "architect" => tier.magenta().bold(),
        "specialist" => tier.cyan(),
        "worker" => tier.white(),
        _ => tier.white(),
    }
}

/// Returns a colored string for memory tier values.
///
/// Semantic = green, Episodic = yellow, Working = dim.
pub fn colorize_memory_tier(tier: &str) -> colored::ColoredString {
    match tier.to_lowercase().as_str() {
        "semantic" => tier.green().bold(),
        "episodic" => tier.yellow(),
        "working" => tier.dimmed(),
        _ => tier.white(),
    }
}

/// Styled label for detail views (bold + dimmed colon).
pub fn label(name: &str) -> String {
    format!("{}{}", name.bold(), ":".dimmed())
}

/// Section header with underline.
pub fn section_header(title: &str) -> String {
    format!("\n{}", title.bold().underline())
}

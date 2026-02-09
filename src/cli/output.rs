//! Output formatting utilities for the CLI.

use serde::Serialize;

pub trait CommandOutput: Serialize {
    fn to_human(&self) -> String;
    fn to_json(&self) -> serde_json::Value;
}

pub fn output<T: CommandOutput>(result: &T, json_mode: bool) {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result.to_json()).unwrap_or_default());
    } else {
        println!("{}", result.to_human());
    }
}

/// Truncate a string to a maximum length, appending "..." if truncated.
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

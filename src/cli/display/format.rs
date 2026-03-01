//! ID, time, and truncation formatters for CLI output.

use chrono::{DateTime, Utc};
use chrono_humanize::HumanTime;

/// Return first 8 chars of a UUID string for list display.
pub fn short_id(id: &str) -> &str {
    if id.len() >= 8 {
        &id[..8]
    } else {
        id
    }
}

/// Format a DateTime as relative time ("2 hours ago", "3 days ago").
pub fn relative_time(dt: &DateTime<Utc>) -> String {
    let ht = HumanTime::from(*dt - Utc::now());
    ht.to_string()
}

/// Format an optional DateTime as relative time or "-".
pub fn relative_time_opt(dt: Option<&DateTime<Utc>>) -> String {
    match dt {
        Some(dt) => relative_time(dt),
        None => "-".to_string(),
    }
}

/// Format an optional ISO string timestamp as relative time.
///
/// Used when the command already has stringified timestamps.
pub fn relative_time_str(iso: &str) -> String {
    match iso.parse::<DateTime<Utc>>() {
        Ok(dt) => relative_time(&dt),
        Err(_) => iso.to_string(),
    }
}

/// Truncate a string with unicode ellipsis.
pub fn truncate_ellipsis(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}\u{2026}", &s[..max_len.saturating_sub(1)])
    }
}

/// Format a count with optional label: "3 tools", "0 constraints".
pub fn count_label(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("{} {}", n, singular)
    } else {
        format!("{} {}", n, plural)
    }
}

//! ID, time, and truncation formatters for CLI output.

use chrono::{DateTime, Utc};
use chrono_humanize::HumanTime;

/// Return first 8 chars of a UUID string for list display.
pub fn short_id(id: &str) -> &str {
    if id.len() >= 8 { &id[..8] } else { id }
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
        let boundary = s
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i < max_len.saturating_sub(1))
            .last()
            .unwrap_or(0);
        format!("{}\u{2026}", &s[..boundary])
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

/// Parse a human-friendly duration string like "7d", "24h", "1w", "30m" into a
/// `chrono::Duration`.
pub fn parse_duration(s: &str) -> anyhow::Result<chrono::Duration> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("duration string cannot be empty");
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let value: i64 = num_str.parse().map_err(|_| {
        anyhow::anyhow!(
            "invalid duration '{}': expected a number followed by a unit (d/h/w/m)",
            s
        )
    })?;

    match unit {
        "m" => Ok(chrono::Duration::minutes(value)),
        "h" => Ok(chrono::Duration::hours(value)),
        "d" => Ok(chrono::Duration::days(value)),
        "w" => Ok(chrono::Duration::weeks(value)),
        _ => anyhow::bail!(
            "unknown duration unit '{}' in '{}': expected one of m (minutes), h (hours), d (days), w (weeks)",
            unit,
            s
        ),
    }
}

/// Parse a human-friendly duration string like "7d", "24h", "30m" into a
/// `std::time::Duration`.
pub fn parse_std_duration(s: &str) -> anyhow::Result<std::time::Duration> {
    use anyhow::Context;
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("Empty duration string");
    }
    let (num_str, suffix) = if let Some(prefix) = s.strip_suffix('d') {
        (prefix, "d")
    } else if let Some(prefix) = s.strip_suffix('h') {
        (prefix, "h")
    } else if let Some(prefix) = s.strip_suffix('m') {
        (prefix, "m")
    } else {
        anyhow::bail!("Duration must end with 'd', 'h', or 'm' (e.g., '7d', '24h', '30m')");
    };
    let num: u64 = num_str.parse().context("Invalid number in duration")?;
    match suffix {
        "d" => Ok(std::time::Duration::from_secs(num * 86400)),
        "h" => Ok(std::time::Duration::from_secs(num * 3600)),
        "m" => Ok(std::time::Duration::from_secs(num * 60)),
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_days() {
        let d = parse_duration("7d").unwrap();
        assert_eq!(d, chrono::Duration::days(7));
    }

    #[test]
    fn test_parse_duration_hours() {
        let d = parse_duration("24h").unwrap();
        assert_eq!(d, chrono::Duration::hours(24));
    }

    #[test]
    fn test_parse_duration_weeks() {
        let d = parse_duration("2w").unwrap();
        assert_eq!(d, chrono::Duration::weeks(2));
    }

    #[test]
    fn test_parse_duration_minutes() {
        let d = parse_duration("30m").unwrap();
        assert_eq!(d, chrono::Duration::minutes(30));
    }

    #[test]
    fn test_parse_duration_invalid_unit() {
        assert!(parse_duration("7x").is_err());
    }

    #[test]
    fn test_parse_duration_empty() {
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_parse_duration_no_number() {
        assert!(parse_duration("d").is_err());
    }

    #[test]
    fn test_parse_std_duration_days() {
        let d = parse_std_duration("7d").unwrap();
        assert_eq!(d, std::time::Duration::from_secs(7 * 86400));
    }

    #[test]
    fn test_parse_std_duration_hours() {
        let d = parse_std_duration("24h").unwrap();
        assert_eq!(d, std::time::Duration::from_secs(24 * 3600));
    }

    #[test]
    fn test_parse_std_duration_minutes() {
        let d = parse_std_duration("30m").unwrap();
        assert_eq!(d, std::time::Duration::from_secs(30 * 60));
    }

    #[test]
    fn test_parse_std_duration_invalid() {
        assert!(parse_std_duration("").is_err());
        assert!(parse_std_duration("7x").is_err());
    }
}

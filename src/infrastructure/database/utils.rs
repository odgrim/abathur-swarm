//! Database utility functions
//!
//! Common utilities for database operations including datetime parsing.

use chrono::{DateTime, NaiveDateTime, Utc};

/// Parse datetime from multiple formats (RFC3339 and SQLite default format)
///
/// This function handles datetime strings in various formats to ensure compatibility
/// with both properly formatted RFC3339 timestamps and SQLite's default datetime format.
///
/// Supports:
/// - RFC3339: "2025-10-29T17:28:13Z", "2025-10-29T17:28:13+00:00"
/// - SQLite default: "2025-10-29 17:28:13"
/// - ISO 8601 without timezone: "2025-10-29T17:28:13"
///
/// # Arguments
/// * `s` - The datetime string to parse
///
/// # Returns
/// * `Ok(DateTime<Utc>)` - Successfully parsed datetime in UTC
/// * `Err(chrono::ParseError)` - If parsing fails for all supported formats
///
/// # Examples
/// ```
/// use abathur_cli::infrastructure::database::utils::parse_datetime;
///
/// // RFC3339 format
/// let dt1 = parse_datetime("2025-10-29T17:28:13Z").unwrap();
///
/// // SQLite format
/// let dt2 = parse_datetime("2025-10-29 17:28:13").unwrap();
///
/// assert_eq!(dt1, dt2);
/// ```
pub fn parse_datetime(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    // Try RFC3339 first (preferred format)
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Try SQLite's default datetime format: "YYYY-MM-DD HH:MM:SS"
    if let Ok(naive_dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc));
    }

    // Try ISO 8601 without timezone: "YYYY-MM-DDTHH:MM:SS"
    if let Ok(naive_dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc));
    }

    // Return RFC3339 error if all parsing attempts fail
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rfc3339() {
        let dt = parse_datetime("2025-10-29T17:28:13Z").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-10-29T17:28:13+00:00");
    }

    #[test]
    fn test_parse_rfc3339_with_offset() {
        let dt = parse_datetime("2025-10-29T17:28:13+00:00").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-10-29T17:28:13+00:00");
    }

    #[test]
    fn test_parse_sqlite_format() {
        let dt = parse_datetime("2025-10-29 17:28:13").unwrap();
        // SQLite format is interpreted as UTC
        assert_eq!(dt.to_rfc3339(), "2025-10-29T17:28:13+00:00");
    }

    #[test]
    fn test_parse_iso8601_no_timezone() {
        let dt = parse_datetime("2025-10-29T17:28:13").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-10-29T17:28:13+00:00");
    }

    #[test]
    fn test_parse_invalid_format() {
        let result = parse_datetime("invalid datetime");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_string() {
        let result = parse_datetime("");
        assert!(result.is_err());
    }
}

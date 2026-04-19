//! Cost window service for quiet-hours scheduling.
//!
//! Determines whether the swarm is currently inside a quiet window
//! by evaluating cron expressions against the current time. When inside
//! a quiet window, the swarm suppresses dispatching new work.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use tokio::sync::RwLock;

use crate::domain::models::quiet_window::{QuietWindow, QuietWindowStatus};
use crate::domain::ports::quiet_window_repository::QuietWindowRepository;

/// Result of a quiet-window evaluation.
#[derive(Debug, Clone)]
pub struct QuietWindowCheck {
    /// Whether we are currently inside any quiet window.
    pub is_quiet: bool,
    /// The name of the active quiet window, if any.
    pub active_window_name: Option<String>,
    /// The active window's ID, if any.
    pub active_window_id: Option<uuid::Uuid>,
}

/// Service that manages quiet-window evaluation.
///
/// Caches enabled windows in memory and re-evaluates on demand.
pub struct CostWindowService {
    repo: Arc<dyn QuietWindowRepository>,
    /// Cached copy of enabled windows, refreshed via `reload_windows()`.
    windows: RwLock<Vec<QuietWindow>>,
}

impl CostWindowService {
    /// Create a new service from a repository.
    pub fn new(repo: Arc<dyn QuietWindowRepository>) -> Self {
        Self {
            repo,
            windows: RwLock::new(Vec::new()),
        }
    }

    /// Reload enabled windows from the database into the cache.
    pub async fn reload_windows(&self) -> Result<usize, String> {
        let windows = self
            .repo
            .list_enabled()
            .await
            .map_err(|e| format!("failed to load quiet windows: {}", e))?;
        let count = windows.len();
        *self.windows.write().await = windows;
        Ok(count)
    }

    /// Check whether the current time falls inside any enabled quiet window.
    pub async fn is_in_quiet_window(&self) -> QuietWindowCheck {
        self.check_at(Utc::now()).await
    }

    /// Check whether a specific time falls inside any enabled quiet window.
    ///
    /// Algorithm for each window:
    /// 1. Parse `start_cron` and `end_cron` as cron schedules.
    /// 2. Find the most recent fire time of `start_cron` before `now` → `last_start`.
    /// 3. Find the most recent fire time of `end_cron` before `now` → `last_end`.
    /// 4. If `last_start > last_end`, we are inside this window.
    pub async fn check_at(&self, now: DateTime<Utc>) -> QuietWindowCheck {
        let windows = self.windows.read().await;
        for w in windows.iter() {
            if w.status != QuietWindowStatus::Enabled {
                continue;
            }
            if self.is_inside_window(w, now) {
                return QuietWindowCheck {
                    is_quiet: true,
                    active_window_name: Some(w.name.clone()),
                    active_window_id: Some(w.id),
                };
            }
        }
        QuietWindowCheck {
            is_quiet: false,
            active_window_name: None,
            active_window_id: None,
        }
    }

    /// Determine whether `now` falls inside the given window.
    fn is_inside_window(&self, window: &QuietWindow, now: DateTime<Utc>) -> bool {
        // Parse the cron schedules using the `cron` crate.
        let start_schedule =
            match cron::Schedule::from_str(&normalize_cron_5to7(&window.start_cron)) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        window_name = %window.name,
                        error = %e,
                        "invalid start_cron expression, skipping window"
                    );
                    return false;
                }
            };
        let end_schedule = match cron::Schedule::from_str(&normalize_cron_5to7(&window.end_cron)) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    window_name = %window.name,
                    error = %e,
                    "invalid end_cron expression, skipping window"
                );
                return false;
            }
        };

        // Convert `now` to the window's timezone so that cron expressions like
        // "0 5 * * *" are evaluated in the configured timezone, not UTC.
        let tz: Tz = match window.timezone.parse() {
            Ok(tz) => tz,
            Err(_) => {
                tracing::warn!(
                    window_name = %window.name,
                    timezone = %window.timezone,
                    "invalid timezone, falling back to UTC"
                );
                chrono_tz::UTC
            }
        };
        let now_in_tz = now.with_timezone(&tz);
        let last_start = last_fire_before_tz(&start_schedule, now_in_tz);
        let last_end = last_fire_before_tz(&end_schedule, now_in_tz);

        match (last_start, last_end) {
            (Some(ls), Some(le)) => ls > le,
            // If start has fired but end never has, we're inside the first window ever
            (Some(_), None) => true,
            // If end has fired but start never has, we're outside
            (None, Some(_)) => false,
            // Neither has ever fired
            (None, None) => false,
        }
    }

    /// List all windows (from DB, not cache).
    pub async fn list_windows(&self) -> Result<Vec<QuietWindow>, String> {
        use crate::domain::ports::quiet_window_repository::QuietWindowFilter;
        self.repo
            .list(QuietWindowFilter::default())
            .await
            .map_err(|e| format!("failed to list quiet windows: {}", e))
    }
}

/// Convert a 5-field cron expression (min hour dom month dow) to the 7-field
/// format expected by the `cron` crate (sec min hour dom month dow year).
fn normalize_cron_5to7(expr: &str) -> String {
    let trimmed = expr.trim();
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    if fields.len() == 5 {
        // Prepend seconds=0 and append year=*
        format!("0 {} *", trimmed)
    } else if fields.len() == 6 {
        // Already has seconds, append year=*
        format!("{} *", trimmed)
    } else {
        // Already 7 fields or malformed — pass through
        trimmed.to_string()
    }
}

/// Find the most recent fire time of a cron schedule strictly before `before`,
/// evaluated in the given timezone.
///
/// The `cron` crate only provides `after()` iterators, so we search backwards
/// by iterating from a reasonable past start point. We look back up to 8 days
/// which covers all reasonable cron patterns (weekly is the longest period).
fn last_fire_before_tz<T: chrono::TimeZone>(
    schedule: &cron::Schedule,
    before: DateTime<T>,
) -> Option<DateTime<T>> {
    let search_start = before.clone() - chrono::Duration::days(8);
    let mut last = None;
    for fire_time in schedule.after(&search_start) {
        if fire_time >= before {
            break;
        }
        last = Some(fire_time);
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_normalize_cron_5to7() {
        assert_eq!(normalize_cron_5to7("0 9 * * 1-5"), "0 0 9 * * 1-5 *");
        assert_eq!(normalize_cron_5to7("0 0 17 * * 1-5"), "0 0 17 * * 1-5 *");
        assert_eq!(normalize_cron_5to7("0 0 9 * * 1-5 *"), "0 0 9 * * 1-5 *");
    }

    #[test]
    fn test_last_fire_before_tz() {
        // Cron: every hour at minute 0 → "0 0 * * * * *"
        let schedule = cron::Schedule::from_str("0 0 * * * * *").unwrap();
        // At 14:30 UTC, last fire should be 14:00
        let now = Utc.with_ymd_and_hms(2026, 3, 28, 14, 30, 0).unwrap();
        let last = last_fire_before_tz(&schedule, now);
        assert!(last.is_some());
        let last = last.unwrap();
        assert_eq!(last.hour(), 14);
        assert_eq!(last.minute(), 0);
    }

    #[test]
    fn test_timezone_aware_quiet_window() {
        // Regression test: quiet window with America/Los_Angeles timezone
        // should evaluate cron in that timezone, not UTC.
        //
        // Window: 5am-6pm Pacific (suppress during daytime)
        let window = QuietWindow::new(
            "peak",
            "Suppress during peak hours",
            "0 5 * * *",
            "0 18 * * *",
            "America/Los_Angeles",
        );

        let service = CostWindowService {
            repo: Arc::new(NullQuietWindowRepo),
            windows: RwLock::new(vec![]),
        };

        // 10:30pm PDT = 5:30am UTC next day
        // This is OUTSIDE the quiet window (nighttime in Pacific)
        let night_pdt = Utc.with_ymd_and_hms(2026, 3, 30, 5, 30, 0).unwrap();
        assert!(
            !service.is_inside_window(&window, night_pdt),
            "10:30pm PDT should be outside the 5am-6pm Pacific quiet window"
        );

        // 12:00pm PDT = 7:00pm UTC
        // This is INSIDE the quiet window (midday in Pacific)
        let midday_pdt = Utc.with_ymd_and_hms(2026, 3, 30, 19, 0, 0).unwrap();
        assert!(
            service.is_inside_window(&window, midday_pdt),
            "12:00pm PDT should be inside the 5am-6pm Pacific quiet window"
        );
    }

    #[test]
    fn test_is_inside_window_logic() {
        // Window: 9am-5pm weekdays
        // start_cron: "0 9 * * 1-5" (5-field) → fires at 9:00
        // end_cron:   "0 17 * * 1-5" (5-field) → fires at 17:00
        let window = QuietWindow::new(
            "business-hours",
            "Suppress during business hours",
            "0 9 * * 1-5",
            "0 17 * * 1-5",
            "UTC",
        );

        let service = CostWindowService {
            repo: Arc::new(NullQuietWindowRepo),
            windows: RwLock::new(vec![]),
        };

        // Wednesday 2026-03-25 at 12:00 UTC → inside window (9am fire > no 17 fire yet today)
        let inside_time = Utc.with_ymd_and_hms(2026, 3, 25, 12, 0, 0).unwrap();
        assert!(service.is_inside_window(&window, inside_time));

        // Wednesday 2026-03-25 at 18:00 UTC → outside window (17:00 fire > 9:00 fire)
        let outside_time = Utc.with_ymd_and_hms(2026, 3, 25, 18, 0, 0).unwrap();
        assert!(!service.is_inside_window(&window, outside_time));

        // Wednesday 2026-03-25 at 08:00 UTC → outside window
        // Last start was previous day's 9am, last end was previous day's 17pm
        // So last_end > last_start → outside
        let early_time = Utc.with_ymd_and_hms(2026, 3, 25, 8, 0, 0).unwrap();
        assert!(!service.is_inside_window(&window, early_time));
    }

    use chrono::Timelike;

    /// Null repository for testing (never called).
    struct NullQuietWindowRepo;

    #[async_trait::async_trait]
    impl QuietWindowRepository for NullQuietWindowRepo {
        async fn create(&self, _: &QuietWindow) -> crate::domain::errors::DomainResult<()> {
            Ok(())
        }
        async fn get(
            &self,
            _: uuid::Uuid,
        ) -> crate::domain::errors::DomainResult<Option<QuietWindow>> {
            Ok(None)
        }
        async fn get_by_name(
            &self,
            _: &str,
        ) -> crate::domain::errors::DomainResult<Option<QuietWindow>> {
            Ok(None)
        }
        async fn update(&self, _: &QuietWindow) -> crate::domain::errors::DomainResult<()> {
            Ok(())
        }
        async fn delete(&self, _: uuid::Uuid) -> crate::domain::errors::DomainResult<()> {
            Ok(())
        }
        async fn list(
            &self,
            _: crate::domain::ports::quiet_window_repository::QuietWindowFilter,
        ) -> crate::domain::errors::DomainResult<Vec<QuietWindow>> {
            Ok(vec![])
        }
        async fn list_enabled(&self) -> crate::domain::errors::DomainResult<Vec<QuietWindow>> {
            Ok(vec![])
        }
    }
}

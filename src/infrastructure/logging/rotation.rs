//! Log file rotation and cleanup
//!
//! Provides automatic log rotation based on:
//! - File size limits
//! - Time-based retention policies
//!
//! Rotated files are renamed with timestamps for archival

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Log file rotator with size and time-based policies
#[derive(Debug, Clone)]
pub struct LogRotator {
    /// Number of days to retain old log files
    retention_days: u32,
    /// Maximum file size in bytes before rotation
    max_file_size: u64,
}

impl LogRotator {
    /// Create a new log rotator
    ///
    /// # Arguments
    /// * `retention_days` - Number of days to keep old log files
    /// * `max_file_size` - Maximum file size in bytes before rotation
    pub fn new(retention_days: u32, max_file_size: u64) -> Self {
        Self {
            retention_days,
            max_file_size,
        }
    }

    /// Check if a log file needs rotation based on size
    ///
    /// Returns `true` if the file exists and exceeds `max_file_size`
    pub async fn should_rotate(&self, log_path: impl AsRef<Path>) -> Result<bool> {
        let log_path = log_path.as_ref();

        if !log_path.exists() {
            return Ok(false);
        }

        let metadata = tokio::fs::metadata(log_path)
            .await
            .context("failed to get log file metadata")?;

        let size = metadata.len();

        debug!(
            path = %log_path.display(),
            size = size,
            max_size = self.max_file_size,
            "checking if log rotation needed"
        );

        Ok(size >= self.max_file_size)
    }

    /// Rotate a log file by renaming it with a timestamp
    ///
    /// The file is renamed to `<original_name>.<timestamp>`
    /// A new empty file is NOT created - the caller should handle that
    ///
    /// # Returns
    /// Path to the rotated file
    pub async fn rotate_if_needed(&self, log_path: impl AsRef<Path>) -> Result<()> {
        let log_path = log_path.as_ref();

        if !self.should_rotate(log_path).await? {
            return Ok(());
        }

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");

        // Determine rotated filename
        let rotated_path = if let Some(ext) = log_path.extension() {
            log_path.with_extension(format!("{}.{}", ext.to_string_lossy(), timestamp))
        } else {
            PathBuf::from(format!("{}.{}", log_path.display(), timestamp))
        };

        // Rename the file
        tokio::fs::rename(log_path, &rotated_path)
            .await
            .context("failed to rotate log file")?;

        info!(
            old_path = %log_path.display(),
            new_path = %rotated_path.display(),
            "rotated log file"
        );

        Ok(())
    }

    /// Clean up old log files beyond retention period
    ///
    /// Searches the log directory for files matching the log file pattern
    /// and deletes those older than `retention_days`
    ///
    /// # Returns
    /// Number of files deleted
    pub async fn cleanup_old_logs(&self, log_dir: impl AsRef<Path>) -> Result<usize> {
        let log_dir = log_dir.as_ref();

        if !log_dir.exists() {
            warn!(path = %log_dir.display(), "log directory does not exist");
            return Ok(0);
        }

        let cutoff = Utc::now() - Duration::days(i64::from(self.retention_days));
        let mut deleted_count = 0;

        // Read directory entries
        let mut entries = tokio::fs::read_dir(log_dir)
            .await
            .context("failed to read log directory")?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .context("failed to read directory entry")?
        {
            let path = entry.path();

            // Only process .log files (including rotated ones with timestamps)
            if let Some(ext_str) = path.extension().and_then(|s| s.to_str()) {
                if !ext_str.starts_with("log") {
                    continue;
                }
            } else {
                continue;
            }

            // Check file modification time
            let metadata = tokio::fs::metadata(&path)
                .await
                .context("failed to get file metadata")?;

            let modified = metadata
                .modified()
                .context("failed to get file modification time")?;

            let modified_dt: DateTime<Utc> = modified.into();

            if modified_dt < cutoff {
                tokio::fs::remove_file(&path)
                    .await
                    .context("failed to delete old log file")?;

                info!(path = %path.display(), age_days = (Utc::now() - modified_dt).num_days(), "deleted old log file");
                deleted_count += 1;
            }
        }

        if deleted_count > 0 {
            info!(count = deleted_count, "cleaned up old log files");
        }

        Ok(deleted_count)
    }

    /// Run periodic cleanup on a schedule
    ///
    /// This is a long-running async task that should be spawned
    /// It runs cleanup every `interval` duration
    pub async fn run_periodic_cleanup(
        &self,
        log_dir: impl AsRef<Path>,
        interval: std::time::Duration,
    ) -> Result<()> {
        let log_dir = log_dir.as_ref().to_path_buf();
        let mut interval_timer = tokio::time::interval(interval);

        loop {
            interval_timer.tick().await;

            match self.cleanup_old_logs(&log_dir).await {
                Ok(count) => {
                    if count > 0 {
                        info!(count = count, "periodic cleanup completed");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "failed to run periodic cleanup");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;
    use tokio::time::{sleep, Duration as TokioDuration};

    #[tokio::test]
    async fn test_should_rotate_when_file_exceeds_size() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        // Create a file larger than max_file_size
        let mut file = std::fs::File::create(&log_path).unwrap();
        file.write_all(&vec![0u8; 2048]).unwrap();
        drop(file);

        let rotator = LogRotator::new(30, 1024);
        assert!(rotator.should_rotate(&log_path).await.unwrap());
    }

    #[tokio::test]
    async fn test_should_not_rotate_when_file_under_size() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        // Create a small file
        let mut file = std::fs::File::create(&log_path).unwrap();
        file.write_all(b"small content").unwrap();
        drop(file);

        let rotator = LogRotator::new(30, 1024);
        assert!(!rotator.should_rotate(&log_path).await.unwrap());
    }

    #[tokio::test]
    async fn test_should_not_rotate_when_file_missing() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("nonexistent.log");

        let rotator = LogRotator::new(30, 1024);
        assert!(!rotator.should_rotate(&log_path).await.unwrap());
    }

    #[tokio::test]
    async fn test_rotate_if_needed_renames_file() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        // Create a file larger than max_file_size
        let mut file = std::fs::File::create(&log_path).unwrap();
        file.write_all(&vec![0u8; 2048]).unwrap();
        drop(file);

        let rotator = LogRotator::new(30, 1024);
        rotator.rotate_if_needed(&log_path).await.unwrap();

        // Original file should not exist
        assert!(!log_path.exists());

        // A rotated file should exist
        let entries: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();

        assert_eq!(entries.len(), 1);
        let rotated = &entries[0];
        assert!(rotated
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("test.log."));
    }

    #[tokio::test]
    async fn test_rotate_if_needed_does_nothing_when_small() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        // Create a small file
        std::fs::write(&log_path, b"small").unwrap();

        let rotator = LogRotator::new(30, 1024);
        rotator.rotate_if_needed(&log_path).await.unwrap();

        // File should still exist and not be rotated
        assert!(log_path.exists());
        let entries: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .collect();
        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn test_cleanup_old_logs_deletes_expired_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create some log files
        std::fs::write(temp_dir.path().join("old.log"), b"old").unwrap();
        std::fs::write(temp_dir.path().join("app.log.20240101_120000"), b"old").unwrap();
        std::fs::write(temp_dir.path().join("recent.log"), b"recent").unwrap();

        // Set old files to very old modification time (using system touch if available)
        // For testing, we'll use a rotator with 0 retention days
        let rotator = LogRotator::new(0, 1024);

        // Wait a tiny bit to ensure files have different timestamps
        sleep(TokioDuration::from_millis(10)).await;

        let _deleted = rotator.cleanup_old_logs(temp_dir.path()).await.unwrap();

        // Should delete all .log files when retention is 0
        assert!(_deleted >= 1);
    }

    #[tokio::test]
    async fn test_cleanup_old_logs_ignores_non_log_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create mixed file types
        std::fs::write(temp_dir.path().join("data.txt"), b"text").unwrap();
        std::fs::write(temp_dir.path().join("app.json"), b"json").unwrap();
        std::fs::write(temp_dir.path().join("test.log"), b"log").unwrap();

        let rotator = LogRotator::new(0, 1024);
        sleep(TokioDuration::from_millis(10)).await;

        let _deleted = rotator.cleanup_old_logs(temp_dir.path()).await.unwrap();

        // Should only delete .log files
        assert!(temp_dir.path().join("data.txt").exists());
        assert!(temp_dir.path().join("app.json").exists());
    }

    #[tokio::test]
    async fn test_cleanup_handles_missing_directory() {
        let temp_dir = TempDir::new().unwrap();
        let missing_dir = temp_dir.path().join("nonexistent");

        let rotator = LogRotator::new(30, 1024);
        let result = rotator.cleanup_old_logs(&missing_dir).await;

        // Should return Ok(0) for missing directory
        assert_eq!(result.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_rotator_with_different_retention() {
        let rotator1 = LogRotator::new(7, 1024);
        let rotator2 = LogRotator::new(30, 1024);

        assert_eq!(rotator1.retention_days, 7);
        assert_eq!(rotator2.retention_days, 30);
    }

    #[tokio::test]
    async fn test_rotator_with_different_max_size() {
        let rotator1 = LogRotator::new(30, 1024 * 1024); // 1 MB
        let rotator2 = LogRotator::new(30, 10 * 1024 * 1024); // 10 MB

        assert_eq!(rotator1.max_file_size, 1024 * 1024);
        assert_eq!(rotator2.max_file_size, 10 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_rotation_preserves_file_extension() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("app.log");

        // Create large file
        let mut file = std::fs::File::create(&log_path).unwrap();
        file.write_all(&vec![0u8; 2048]).unwrap();
        drop(file);

        let rotator = LogRotator::new(30, 1024);
        rotator.rotate_if_needed(&log_path).await.unwrap();

        // Find rotated file
        let entries: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();

        // Should have pattern like "app.log.20251025_123456"
        assert!(entries[0].starts_with("app.log."));
        assert!(entries[0].len() > "app.log.".len());
    }

    #[tokio::test]
    async fn test_multiple_rotations() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("app.log");

        let rotator = LogRotator::new(30, 1024);

        // Rotate 3 times
        for _i in 0..3 {
            // Create large file
            std::fs::write(&log_path, vec![0u8; 2048]).unwrap();

            rotator.rotate_if_needed(&log_path).await.unwrap();

            // Delay to ensure different timestamps (format is YYYYMMDD_HHMMSS)
            sleep(TokioDuration::from_secs(1)).await;
        }

        // Should have 3 rotated files
        let entries: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .collect();

        assert_eq!(entries.len(), 3);
    }
}

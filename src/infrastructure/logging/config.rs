use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Output format (json, pretty)
    #[serde(default = "default_format")]
    pub format: LogFormat,

    /// Directory for log files (optional, if None logs only to stdout)
    pub log_dir: Option<PathBuf>,

    /// Enable stdout logging
    #[serde(default = "default_true")]
    pub enable_stdout: bool,

    /// Log rotation policy
    #[serde(default)]
    pub rotation: RotationPolicy,

    /// Log retention in days
    #[serde(default = "default_retention_days")]
    pub retention_days: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Pretty,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RotationPolicy {
    Daily,
    Hourly,
    Never,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_format(),
            log_dir: None,
            enable_stdout: true,
            rotation: RotationPolicy::default(),
            retention_days: default_retention_days(),
        }
    }
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self::Daily
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_format() -> LogFormat {
    LogFormat::Json
}

fn default_true() -> bool {
    true
}

fn default_retention_days() -> i64 {
    30
}

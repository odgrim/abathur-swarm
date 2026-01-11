//! Configuration management for the Abathur swarm system.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Configuration file not found: {0}")]
    FileNotFound(String),
    #[error("Failed to read configuration: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("Failed to parse configuration: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("Validation failed for {field}: {reason}")]
    ValidationError { field: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub limits: LimitsConfig,
    pub memory: MemoryConfig,
    pub worktrees: WorktreeConfig,
    pub a2a: A2AConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitsConfig {
    pub max_depth: u32,
    pub max_subtasks: u32,
    pub max_descendants: u32,
    pub max_concurrent_tasks: u32,
    pub max_retries: u32,
    pub task_timeout_secs: u64,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_subtasks: 10,
            max_descendants: 100,
            max_concurrent_tasks: 5,
            max_retries: 3,
            task_timeout_secs: 300,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub decay_rate: f64,
    pub prune_threshold: f64,
    pub maintenance_interval_secs: u64,
    pub max_per_namespace: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            decay_rate: 0.05,
            prune_threshold: 0.1,
            maintenance_interval_secs: 3600,
            max_per_namespace: 10000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreeConfig {
    pub base_path: String,
    pub auto_cleanup: bool,
    pub max_age_hours: u64,
    pub enabled: bool,
    pub branch_prefix: String,
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            base_path: ".abathur/worktrees".to_string(),
            auto_cleanup: true,
            max_age_hours: 168,
            enabled: true,
            branch_prefix: "abathur/task".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct A2AConfig {
    pub gateway_port: u16,
    pub request_timeout_secs: u64,
    pub max_message_size: usize,
    pub enabled: bool,
}

impl Default for A2AConfig {
    fn default() -> Self {
        Self {
            gateway_port: 8080,
            request_timeout_secs: 30,
            max_message_size: 1024 * 1024,
            enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub path: String,
    pub max_connections: u32,
    pub connect_timeout_secs: u64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: ".abathur/abathur.db".to_string(),
            max_connections: 5,
            connect_timeout_secs: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "pretty".to_string(),
        }
    }
}

impl Config {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(ConfigError::FileNotFound(path.display().to_string()));
        }
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;
        config.apply_env_overrides();
        config.validate()?;
        Ok(config)
    }

    pub fn load() -> Result<Self, ConfigError> {
        let path = Path::new("abathur.toml");
        if path.exists() {
            Self::from_file(path)
        } else {
            let mut config = Config::default();
            config.apply_env_overrides();
            config.validate()?;
            Ok(config)
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(val) = std::env::var("ABATHUR_LIMITS_MAX_DEPTH") {
            if let Ok(v) = val.parse() { self.limits.max_depth = v; }
        }
        if let Ok(val) = std::env::var("ABATHUR_DATABASE_PATH") {
            self.database.path = val;
        }
        if let Ok(val) = std::env::var("ABATHUR_LOG_LEVEL") {
            self.logging.level = val;
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.limits.max_depth == 0 {
            return Err(ConfigError::ValidationError {
                field: "limits.max_depth".to_string(),
                reason: "must be greater than 0".to_string(),
            });
        }
        if self.memory.decay_rate < 0.0 || self.memory.decay_rate > 1.0 {
            return Err(ConfigError::ValidationError {
                field: "memory.decay_rate".to_string(),
                reason: "must be between 0.0 and 1.0".to_string(),
            });
        }
        Ok(())
    }

    pub fn sample_toml() -> String {
        toml::to_string_pretty(&Config::default()).unwrap_or_default()
    }
}

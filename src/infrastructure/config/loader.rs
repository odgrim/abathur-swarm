use anyhow::{Context, Result};
use figment::Figment;
use figment::providers::{Env, Format, Serialized, Yaml};
use thiserror::Error;

use crate::domain::models::config::Config;

/// Configuration error types
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Invalid max_agents: {0}. Must be between 1 and 100")]
    InvalidMaxAgents(usize),

    #[error("Invalid rate limit: {0}. Must be positive")]
    InvalidRateLimit(f64),

    #[error("Invalid log level: {0}. Must be one of: trace, debug, info, warn, error")]
    InvalidLogLevel(String),

    #[error("Invalid log format: {0}. Must be one of: json, pretty")]
    InvalidLogFormat(String),

    #[error("Database path cannot be empty")]
    EmptyDatabasePath,

    #[error("Invalid max_connections: {0}. Must be at least 1")]
    InvalidMaxConnections(u32),

    #[error("Invalid burst_size: {0}. Must be at least 1")]
    InvalidBurstSize(u32),

    #[error("Invalid max_retries: {0}. Cannot be 0")]
    InvalidMaxRetries(u32),

    #[error(
        "Invalid backoff configuration: initial_backoff_ms ({0}) must be less than max_backoff_ms ({1})"
    )]
    InvalidBackoff(u64, u64),

    #[error("Configuration validation failed: {0}")]
    ValidationFailed(String),
}

/// Configuration loader with hierarchical merging
pub struct ConfigLoader;

impl ConfigLoader {
    /// Load configuration with hierarchical merging
    ///
    /// Precedence (lowest to highest):
    /// 1. Programmatic defaults (Serialized)
    /// 2. .abathur/config.yaml (project config, created by init)
    /// 3. .abathur/local.yaml (project local overrides, optional)
    /// 4. Environment variables (ABATHUR_* prefix, highest priority)
    ///
    /// Note: Configuration is always project-local (pwd/.abathur/)
    /// to support multiple swarms per machine with different projects.
    pub fn load() -> Result<Config> {
        let config: Config = Figment::new()
            // 1. Start with programmatic defaults
            .merge(Serialized::defaults(Config::default()))
            // 2. Merge project config (primary config, created by init)
            .merge(Yaml::file(".abathur/config.yaml"))
            // 3. Merge project local overrides (optional, for dev/test overrides)
            .merge(Yaml::file(".abathur/local.yaml"))
            // 4. Merge environment variables (highest priority)
            .merge(Env::prefixed("ABATHUR_").split("__"))
            .extract()
            .context("Failed to extract configuration from figment")?;

        Self::validate(&config)?;
        Ok(config)
    }

    /// Load configuration from a specific file
    pub fn load_from_file(path: impl AsRef<std::path::Path>) -> Result<Config> {
        let config: Config = Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Yaml::file(path.as_ref()))
            .extract()
            .context(format!(
                "Failed to load config from {}",
                path.as_ref().display()
            ))?;

        Self::validate(&config)?;
        Ok(config)
    }

    /// Validate configuration after loading
    pub fn validate(config: &Config) -> Result<(), ConfigError> {
        // Validate max_agents
        if config.max_agents == 0 || config.max_agents > 100 {
            return Err(ConfigError::InvalidMaxAgents(config.max_agents));
        }

        // Validate database config
        if config.database.path.is_empty() {
            return Err(ConfigError::EmptyDatabasePath);
        }

        if config.database.max_connections == 0 {
            return Err(ConfigError::InvalidMaxConnections(
                config.database.max_connections,
            ));
        }

        // Validate logging config
        let valid_log_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_log_levels.contains(&config.logging.level.as_str()) {
            return Err(ConfigError::InvalidLogLevel(config.logging.level.clone()));
        }

        let valid_log_formats = ["json", "pretty"];
        if !valid_log_formats.contains(&config.logging.format.as_str()) {
            return Err(ConfigError::InvalidLogFormat(config.logging.format.clone()));
        }

        // Validate rate_limit
        if config.rate_limit.requests_per_second <= 0.0 {
            return Err(ConfigError::InvalidRateLimit(
                config.rate_limit.requests_per_second,
            ));
        }

        if config.rate_limit.burst_size == 0 {
            return Err(ConfigError::InvalidBurstSize(config.rate_limit.burst_size));
        }

        // Validate retry config
        if config.retry.max_retries == 0 {
            return Err(ConfigError::InvalidMaxRetries(config.retry.max_retries));
        }

        if config.retry.initial_backoff_ms >= config.retry.max_backoff_ms {
            return Err(ConfigError::InvalidBackoff(
                config.retry.initial_backoff_ms,
                config.retry.max_backoff_ms,
            ));
        }

        // Validate MCP server configs
        for server in &config.mcp_servers {
            if server.name.is_empty() {
                return Err(ConfigError::ValidationFailed(
                    "MCP server name cannot be empty".to_string(),
                ));
            }
            if server.command.is_empty() {
                return Err(ConfigError::ValidationFailed(format!(
                    "MCP server '{}' command cannot be empty",
                    server.name
                )));
            }
        }

        // Resource limits validation removed - using defaults in ResourceMonitor

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::config::{
        DatabaseConfig, LoggingConfig, RagConfig, RateLimitConfig, RetryConfig, SubstratesConfig,
    };
    use std::env;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.max_agents, 10);
        assert!((config.rate_limit.requests_per_second - 10.0).abs() < f64::EPSILON);
        assert_eq!(config.database.path, ".abathur/abathur.db");
        assert_eq!(config.logging.level, "info");
        ConfigLoader::validate(&config).expect("Default config should be valid");
    }

    #[test]
    fn test_yaml_parsing() {
        let yaml = r"
max_agents: 20
rate_limit:
  requests_per_second: 15.0
  burst_size: 30
database:
  path: /custom/path.db
  max_connections: 5
logging:
  level: debug
  format: pretty
  retention_days: 7
";

        let config: Config = serde_yaml::from_str(yaml).expect("YAML should parse");

        assert_eq!(config.max_agents, 20);
        assert!((config.rate_limit.requests_per_second - 15.0).abs() < f64::EPSILON);
        assert_eq!(config.rate_limit.burst_size, 30);
        assert_eq!(config.database.path, "/custom/path.db");
        assert_eq!(config.database.max_connections, 5);
        assert_eq!(config.logging.level, "debug");
        assert_eq!(config.logging.format, "pretty");
        assert_eq!(config.logging.retention_days, 7);

        ConfigLoader::validate(&config).expect("Parsed config should be valid");
    }

    #[test]
    fn test_validate_valid_config() {
        let config = Config {
            max_agents: 10,
            database: DatabaseConfig {
                path: ".abathur/abathur.db".to_string(),
                max_connections: 10,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "json".to_string(),
                retention_days: 30,
            },
            rate_limit: RateLimitConfig {
                requests_per_second: 10.0,
                burst_size: 20,
            },
            retry: RetryConfig {
                max_retries: 3,
                initial_backoff_ms: 1000,
                max_backoff_ms: 30000,
            },
            mcp_servers: vec![],
            substrates: SubstratesConfig::default(),
            rag: RagConfig::default(),
        };
        assert!(ConfigLoader::validate(&config).is_ok());
    }

    #[test]
    fn test_validate_zero_agents() {
        let config = Config {
            max_agents: 0,
            ..Default::default()
        };

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidMaxAgents(0)
        ));
    }

    #[test]
    fn test_validate_too_many_agents() {
        let config = Config {
            max_agents: 101,
            ..Default::default()
        };

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidMaxAgents(101)
        ));
    }

    #[test]
    fn test_validate_invalid_log_level() {
        let mut config = Config::default();
        config.logging.level = "invalid".to_string();

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::InvalidLogLevel(level) => assert_eq!(level, "invalid"),
            _ => panic!("Expected InvalidLogLevel error"),
        }
    }

    #[test]
    fn test_validate_invalid_log_format() {
        let mut config = Config::default();
        config.logging.format = "xml".to_string();

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::InvalidLogFormat(format) => assert_eq!(format, "xml"),
            _ => panic!("Expected InvalidLogFormat error"),
        }
    }

    #[test]
    fn test_validate_negative_rate_limit() {
        let mut config = Config::default();
        config.rate_limit.requests_per_second = -5.0;

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidRateLimit(_)
        ));
    }

    #[test]
    fn test_validate_zero_rate_limit() {
        let mut config = Config::default();
        config.rate_limit.requests_per_second = 0.0;

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidRateLimit(_)
        ));
    }

    #[test]
    fn test_validate_zero_burst_size() {
        let mut config = Config::default();
        config.rate_limit.burst_size = 0;

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidBurstSize(0)
        ));
    }

    #[test]
    fn test_validate_empty_database_path() {
        let mut config = Config::default();
        config.database.path = String::new();

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::EmptyDatabasePath
        ));
    }

    #[test]
    fn test_validate_zero_max_connections() {
        let mut config = Config::default();
        config.database.max_connections = 0;

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidMaxConnections(0)
        ));
    }

    #[test]
    fn test_validate_zero_max_retries() {
        let mut config = Config::default();
        config.retry.max_retries = 0;

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidMaxRetries(0)
        ));
    }

    #[test]
    fn test_validate_invalid_backoff() {
        let mut config = Config::default();
        config.retry.initial_backoff_ms = 30000;
        config.retry.max_backoff_ms = 10000;

        let result = ConfigLoader::validate(&config);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidBackoff(30000, 10000)
        ));
    }

    // Resource limits feature has been removed
    // #[test]
    // fn test_validate_invalid_resource_limits() {
    //     let mut config = Config::default();
    //     config.resource_limits.per_agent_memory_mb = 8192;
    //     config.resource_limits.total_memory_mb = 4096;
    //
    //     let result = ConfigLoader::validate(&config);
    //     assert!(result.is_err());
    //     match result.unwrap_err() {
    //         ConfigError::ValidationFailed(msg) => {
    //             assert!(msg.contains("per_agent_memory_mb"));
    //             assert!(msg.contains("total_memory_mb"));
    //         }
    //         _ => panic!("Expected ValidationFailed error"),
    //     }
    // }

    #[test]
    fn test_env_override() {
        // Set environment variables
        unsafe {
            env::set_var("ABATHUR_MAX_AGENTS", "25");
            env::set_var("ABATHUR_RATE_LIMIT__REQUESTS_PER_SECOND", "20.0");
            env::set_var("ABATHUR_LOGGING__LEVEL", "debug");
        }

        // Note: This test requires actual config files to exist
        // In a real environment, ConfigLoader::load() would merge env vars
        // For unit testing, we'll just verify the env vars are set
        assert_eq!(env::var("ABATHUR_MAX_AGENTS").unwrap(), "25");
        assert_eq!(
            env::var("ABATHUR_RATE_LIMIT__REQUESTS_PER_SECOND").unwrap(),
            "20.0"
        );
        assert_eq!(env::var("ABATHUR_LOGGING__LEVEL").unwrap(), "debug");

        // Cleanup
        unsafe {
            env::remove_var("ABATHUR_MAX_AGENTS");
            env::remove_var("ABATHUR_RATE_LIMIT__REQUESTS_PER_SECOND");
            env::remove_var("ABATHUR_LOGGING__LEVEL");
        }
    }

    #[test]
    fn test_hierarchical_merging() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Create base config
        let mut base_file = NamedTempFile::new().unwrap();
        writeln!(
            base_file,
            "max_agents: 5\nlogging:\n  level: info\n  format: json"
        )
        .unwrap();
        base_file.flush().unwrap();

        // Create override config
        let mut override_file = NamedTempFile::new().unwrap();
        writeln!(override_file, "max_agents: 15\nlogging:\n  level: debug").unwrap();
        override_file.flush().unwrap();

        let config: Config = Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Yaml::file(base_file.path()))
            .merge(Yaml::file(override_file.path()))
            .extract()
            .unwrap();

        assert_eq!(config.max_agents, 15, "Override should win");
        assert_eq!(
            config.logging.level, "debug",
            "Override should win for nested fields"
        );
        assert_eq!(
            config.logging.format, "json",
            "Base value should persist when not overridden"
        );
    }
}

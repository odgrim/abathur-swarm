//! Federation configuration types.
//!
//! Parsed from the `[federation]` section of `abathur.toml`.

use serde::{Deserialize, Serialize};

/// Role of this swarm in the federation hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FederationRole {
    /// Parent swarm that delegates work to cerebrates.
    #[default]
    Overmind,
    /// Child swarm that receives delegated work.
    Cerebrate,
}

impl std::fmt::Display for FederationRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Overmind => write!(f, "overmind"),
            Self::Cerebrate => write!(f, "cerebrate"),
        }
    }
}

/// Top-level federation configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FederationConfig {
    /// Whether federation is enabled.
    pub enabled: bool,
    /// Role of this swarm.
    pub role: FederationRole,
    /// Unique identifier for this swarm in the federation.
    pub swarm_id: String,
    /// Human-readable name.
    pub display_name: String,
    /// Heartbeat interval in seconds.
    pub heartbeat_interval_secs: u64,
    /// Number of missed heartbeats before marking unreachable.
    pub missed_heartbeat_threshold: u32,
    /// Timeout before orphaned tasks delegated to unreachable cerebrates are failed.
    pub task_orphan_timeout_secs: u64,
    /// Timeout before a stall is detected (no progress received).
    pub stall_timeout_secs: u64,
    /// Port for the federation listener.
    pub port: u16,
    /// TLS configuration.
    pub tls: FederationTlsConfig,
    /// Parent configuration (only used when role = Cerebrate).
    pub parent: Option<FederationParentConfig>,
    /// Cerebrate configurations (only used when role = Overmind).
    pub cerebrates: Vec<CerebrateConfig>,
}

impl Default for FederationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            role: FederationRole::default(),
            swarm_id: uuid::Uuid::new_v4().to_string(),
            display_name: "unnamed-swarm".to_string(),
            heartbeat_interval_secs: 30,
            missed_heartbeat_threshold: 3,
            task_orphan_timeout_secs: 3600,
            stall_timeout_secs: 1800,
            port: 8443,
            tls: FederationTlsConfig::default(),
            parent: None,
            cerebrates: Vec::new(),
        }
    }
}

/// TLS configuration for federation connections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FederationTlsConfig {
    /// Path to the TLS certificate file.
    pub cert_path: Option<String>,
    /// Path to the TLS private key file.
    pub key_path: Option<String>,
    /// Path to the CA certificate file for verifying peers.
    pub ca_path: Option<String>,
    /// Whether to allow self-signed certificates.
    pub allow_self_signed: bool,
}

/// Configuration for the parent swarm (used when this swarm is a Cerebrate).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FederationParentConfig {
    /// Expected parent swarm ID (for validation).
    pub expected_id: Option<String>,
    /// Maximum concurrent tasks to accept from the parent.
    pub max_accepted_tasks: u32,
}

impl Default for FederationParentConfig {
    fn default() -> Self {
        Self {
            expected_id: None,
            max_accepted_tasks: 10,
        }
    }
}

/// Configuration for a cerebrate (child swarm).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CerebrateConfig {
    /// Unique identifier.
    pub id: String,
    /// Human-readable name.
    pub display_name: String,
    /// URL of the cerebrate's federation endpoint.
    pub url: String,
    /// Capabilities this cerebrate offers.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Maximum concurrent delegations to this cerebrate.
    pub max_concurrent_delegations: u32,
    /// Whether to automatically connect on startup.
    #[serde(default)]
    pub auto_connect: bool,
    /// Permissions granted to this cerebrate.
    #[serde(default)]
    pub permissions: Vec<String>,
}

impl Default for CerebrateConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            url: String::new(),
            capabilities: Vec::new(),
            max_concurrent_delegations: 10,
            auto_connect: false,
            permissions: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_federation_config_defaults() {
        let config = FederationConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.role, FederationRole::Overmind);
        assert_eq!(config.heartbeat_interval_secs, 30);
        assert_eq!(config.missed_heartbeat_threshold, 3);
        assert_eq!(config.port, 8443);
        assert!(config.cerebrates.is_empty());
    }

    #[test]
    fn test_federation_config_serde_roundtrip() {
        let config = FederationConfig {
            enabled: true,
            role: FederationRole::Cerebrate,
            swarm_id: "test-swarm".to_string(),
            display_name: "Test Swarm".to_string(),
            heartbeat_interval_secs: 15,
            missed_heartbeat_threshold: 5,
            task_orphan_timeout_secs: 1800,
            stall_timeout_secs: 900,
            port: 9443,
            tls: FederationTlsConfig {
                cert_path: Some("/path/to/cert.pem".to_string()),
                key_path: Some("/path/to/key.pem".to_string()),
                ca_path: None,
                allow_self_signed: true,
            },
            parent: Some(FederationParentConfig {
                expected_id: Some("parent-id".to_string()),
                max_accepted_tasks: 5,
            }),
            cerebrates: Vec::new(),
        };

        let json = serde_json::to_string(&config)
            .expect("roundtrip should succeed: serialize FederationConfig to JSON");
        let roundtrip: FederationConfig = serde_json::from_str(&json)
            .expect("roundtrip should succeed: deserialize FederationConfig from JSON");
        assert_eq!(roundtrip, config);
    }

    #[test]
    fn test_federation_config_toml_roundtrip() {
        let toml_str = r#"
            enabled = true
            role = "overmind"
            swarm_id = "my-swarm"
            display_name = "My Swarm"
            heartbeat_interval_secs = 30
            missed_heartbeat_threshold = 3
            task_orphan_timeout_secs = 3600
            stall_timeout_secs = 1800
            port = 8443

            [tls]
            allow_self_signed = false

            [[cerebrates]]
            id = "c1"
            display_name = "Cerebrate 1"
            url = "https://c1.example.com:8443"
            capabilities = ["rust", "python"]
            max_concurrent_delegations = 5
            auto_connect = true
        "#;

        let config: FederationConfig = toml::from_str(toml_str)
            .expect("roundtrip should succeed: parse FederationConfig from TOML");
        assert!(config.enabled);
        assert_eq!(config.role, FederationRole::Overmind);
        assert_eq!(config.cerebrates.len(), 1);
        assert_eq!(config.cerebrates[0].id, "c1");
        assert!(config.cerebrates[0].auto_connect);
        assert_eq!(config.cerebrates[0].capabilities, vec!["rust", "python"]);
    }

    #[test]
    fn test_cerebrate_config_defaults() {
        let config = CerebrateConfig::default();
        assert_eq!(config.max_concurrent_delegations, 10);
        assert!(!config.auto_connect);
        assert!(config.capabilities.is_empty());
    }

    #[test]
    fn test_federation_role_display() {
        assert_eq!(FederationRole::Overmind.to_string(), "overmind");
        assert_eq!(FederationRole::Cerebrate.to_string(), "cerebrate");
    }

    #[test]
    fn test_malformed_toml_returns_err_without_panic() {
        // Unclosed array / syntactically invalid TOML.
        let malformed = r#"
            enabled = true
            role = "overmind
            cerebrates = [
        "#;
        let result: Result<FederationConfig, _> = toml::from_str(malformed);
        assert!(
            result.is_err(),
            "malformed TOML must return Err, not panic"
        );
    }

    #[test]
    fn test_malformed_json_returns_err_without_panic() {
        // Truncated / syntactically invalid JSON.
        let malformed = r#"{"enabled": true, "role": "overmind","#;
        let result: Result<FederationConfig, _> = serde_json::from_str(malformed);
        assert!(
            result.is_err(),
            "malformed JSON must return Err, not panic"
        );
    }

    #[test]
    fn test_unknown_role_variant_returns_err() {
        // An unknown FederationRole variant must produce Err rather than
        // silently defaulting or panicking.
        let json = r#"{"role":"dictator"}"#;
        let result: Result<FederationRole, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "unknown role variant must return Err, not panic"
        );

        // Same check via the containing config's TOML path.
        let toml_str = r#"
            role = "dictator"
        "#;
        let toml_result: Result<FederationConfig, _> = toml::from_str(toml_str);
        assert!(
            toml_result.is_err(),
            "unknown role variant in TOML must return Err, not panic"
        );
    }
}

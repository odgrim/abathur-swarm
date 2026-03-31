//! Configuration types for the human cerebrate proxy.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub identity: IdentityConfig,
    pub parent: ParentConfig,
    pub clickup: ClickUpConfig,
    pub polling: PollingConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub tls: TlsConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub bind_address: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct IdentityConfig {
    pub cerebrate_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub max_concurrent_tasks: u32,
}

#[derive(Debug, Deserialize)]
pub struct ParentConfig {
    pub overmind_url: String,
    pub heartbeat_interval_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct ClickUpConfig {
    #[expect(dead_code, reason = "reserved for future ClickUp workspace-scoped operations")]
    pub workspace_id: String,
    pub list_id: String,
    pub completed_statuses: Vec<String>,
    pub failed_statuses: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PollingConfig {
    pub interval_secs: u64,
    pub task_deadline_secs: u64,
    pub progress_interval_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct TlsConfig {
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    #[expect(dead_code, reason = "reserved for mTLS client certificate verification")]
    pub ca_path: Option<String>,
    #[serde(default)]
    #[expect(dead_code, reason = "reserved for development TLS configuration")]
    pub allow_self_signed: bool,
}

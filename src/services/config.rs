//! Configuration management for the Abathur swarm system.

use crate::domain::models::workflow_template::WorkflowTemplate;
use crate::services::swarm_orchestrator::PollingConfig;
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

/// Default workflow name.
fn default_workflow_name() -> String {
    "code".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub limits: LimitsConfig,
    pub spawn_limits: SpawnLimitsConfig,
    pub restructure_limits: RestructureLimitsConfig,
    pub evolution: EvolutionConfig,
    pub memory: MemoryConfig,
    pub worktrees: WorktreeConfig,
    pub a2a: A2AConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
    pub polling: PollingConfig,
    /// External adapter subsystem configuration.
    pub adapters: AdapterConfig,
    /// Budget-aware scheduling configuration.
    #[serde(default)]
    pub budget: BudgetConfig,
    /// Name of the default workflow to use.
    #[serde(default = "default_workflow_name")]
    pub default_workflow: String,
    /// User-defined workflow templates.
    #[serde(default)]
    pub workflows: Vec<WorkflowTemplate>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            limits: LimitsConfig::default(),
            spawn_limits: SpawnLimitsConfig::default(),
            restructure_limits: RestructureLimitsConfig::default(),
            evolution: EvolutionConfig::default(),
            memory: MemoryConfig::default(),
            worktrees: WorktreeConfig::default(),
            a2a: A2AConfig::default(),
            database: DatabaseConfig::default(),
            logging: LoggingConfig::default(),
            polling: PollingConfig::default(),
            adapters: AdapterConfig::default(),
            budget: BudgetConfig::default(),
            default_workflow: default_workflow_name(),
            workflows: Vec::new(),
        }
    }
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

/// Configuration for spawn limits (task creation boundaries).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpawnLimitsConfig {
    /// Maximum depth of subtask nesting.
    pub max_subtask_depth: u32,
    /// Maximum number of subtasks per task.
    pub max_subtasks_per_task: u32,
    /// Maximum total descendants from a root task.
    pub max_total_descendants: u32,
    /// Whether to allow extension requests when limits are reached.
    pub allow_limit_extensions: bool,
    /// Maximum extensions allowed per task tree.
    pub max_extensions: u32,
}

impl Default for SpawnLimitsConfig {
    fn default() -> Self {
        Self {
            max_subtask_depth: 5,
            max_subtasks_per_task: 10,
            max_total_descendants: 100,
            allow_limit_extensions: true,
            max_extensions: 2,
        }
    }
}

/// Configuration for DAG restructuring limits (malleability).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestructureLimitsConfig {
    /// Maximum times a task subtree can be restructured.
    pub max_restructure_attempts: u32,
    /// Minimum time between restructure attempts (seconds).
    pub restructure_cooldown_secs: u64,
    /// Maximum depth of restructuring (how many parent levels to consider).
    pub max_restructure_depth: u32,
    /// Whether to allow automatic restructuring on failure.
    pub auto_restructure_on_failure: bool,
    /// Minimum success rate to avoid restructuring trigger.
    pub min_success_rate_for_stability: f64,
}

impl Default for RestructureLimitsConfig {
    fn default() -> Self {
        Self {
            max_restructure_attempts: 3,
            restructure_cooldown_secs: 300,
            max_restructure_depth: 2,
            auto_restructure_on_failure: true,
            min_success_rate_for_stability: 0.5,
        }
    }
}

/// Configuration for evolution (agent template refinement).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EvolutionConfig {
    /// Minimum tasks before evaluating performance.
    pub min_tasks_for_evaluation: u32,
    /// Success rate threshold for triggering minor refinement.
    pub minor_refinement_threshold: f64,
    /// Success rate threshold for triggering major refinement.
    pub major_refinement_threshold: f64,
    /// Success rate threshold for immediate action.
    pub immediate_action_threshold: f64,
    /// Maximum template versions to retain.
    pub max_template_versions: u32,
    /// Whether to enable automatic reversion on regression.
    pub auto_revert_on_regression: bool,
    /// Minimum improvement required to keep a new version.
    pub min_improvement_to_keep: f64,
    /// Window size for computing rolling success rates.
    pub evaluation_window_size: u32,
    /// Cooldown between refinement attempts (seconds).
    pub refinement_cooldown_secs: u64,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            min_tasks_for_evaluation: 10,
            minor_refinement_threshold: 0.6,
            major_refinement_threshold: 0.4,
            immediate_action_threshold: 0.2,
            max_template_versions: 5,
            auto_revert_on_regression: true,
            min_improvement_to_keep: 0.05,
            evaluation_window_size: 20,
            refinement_cooldown_secs: 3600,
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
    /// Federation settings for cross-swarm collaboration.
    pub federation: A2AFederationConfig,
}

impl Default for A2AConfig {
    fn default() -> Self {
        Self {
            gateway_port: 8080,
            request_timeout_secs: 30,
            max_message_size: 1024 * 1024,
            enabled: false,
            federation: A2AFederationConfig::default(),
        }
    }
}

/// Configuration for A2A federation (cross-swarm collaboration).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct A2AFederationConfig {
    /// Whether federation with external swarms is enabled.
    pub enabled: bool,
    /// List of trusted external swarm endpoints.
    pub trusted_swarms: Vec<TrustedSwarmConfig>,
    /// Whether to enable automatic swarm discovery via mDNS or registry.
    pub discovery_enabled: bool,
    /// Discovery registry URL (if using centralized discovery).
    pub discovery_registry_url: Option<String>,
    /// Maximum concurrent requests to external swarms.
    pub max_external_requests: u32,
    /// Rate limit per external swarm (requests per minute).
    pub rate_limit_per_swarm: u32,
    /// Timeout for external swarm requests (seconds).
    pub external_request_timeout_secs: u64,
    /// Whether to accept inbound tasks from external swarms.
    pub allow_inbound_tasks: bool,
    /// Maximum concurrent inbound tasks from external swarms.
    pub max_inbound_tasks: u32,
    /// Authentication method for federation.
    pub auth_method: FederationAuthMethod,
    /// This swarm's public identity for federation.
    pub swarm_identity: Option<SwarmIdentityConfig>,
    /// Capabilities to advertise to other swarms.
    pub advertised_capabilities: Vec<String>,
    /// Task types that can be delegated to external swarms.
    pub delegatable_task_types: Vec<String>,
    /// Whether to require mutual TLS for federation connections.
    pub require_mtls: bool,
}

impl Default for A2AFederationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            trusted_swarms: Vec::new(),
            discovery_enabled: false,
            discovery_registry_url: None,
            max_external_requests: 10,
            rate_limit_per_swarm: 60,
            external_request_timeout_secs: 60,
            allow_inbound_tasks: false,
            max_inbound_tasks: 5,
            auth_method: FederationAuthMethod::default(),
            swarm_identity: None,
            advertised_capabilities: Vec::new(),
            delegatable_task_types: Vec::new(),
            require_mtls: false,
        }
    }
}

/// Configuration for a trusted external swarm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustedSwarmConfig {
    /// Unique identifier for the trusted swarm.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Base URL for the swarm's A2A gateway.
    pub endpoint: String,
    /// Authentication token for this swarm (if using token auth).
    pub auth_token: Option<String>,
    /// Public key for this swarm (if using key-based auth).
    pub public_key: Option<String>,
    /// Trust level (affects what operations are allowed).
    pub trust_level: TrustLevel,
    /// Whether this swarm is currently active.
    pub active: bool,
    /// Capabilities this swarm offers.
    pub capabilities: Vec<String>,
    /// Rate limit override for this specific swarm.
    pub rate_limit_override: Option<u32>,
}

impl Default for TrustedSwarmConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            endpoint: String::new(),
            auth_token: None,
            public_key: None,
            trust_level: TrustLevel::default(),
            active: true,
            capabilities: Vec::new(),
            rate_limit_override: None,
        }
    }
}

/// Trust level for external swarms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Full trust - can delegate any task type.
    Full,
    /// High trust - can delegate most task types.
    High,
    /// Medium trust - can delegate limited task types.
    #[default]
    Medium,
    /// Low trust - read-only collaboration.
    Low,
    /// Untrusted - no collaboration allowed.
    Untrusted,
}

/// Authentication method for federation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FederationAuthMethod {
    /// No authentication (not recommended).
    #[default]
    None,
    /// Pre-shared token authentication.
    Token,
    /// Public/private key pair authentication.
    KeyPair,
    /// Mutual TLS authentication.
    MutualTls,
    /// OAuth2 / OIDC authentication.
    OAuth2 {
        issuer_url: String,
        client_id: String,
        client_secret: Option<String>,
    },
}

/// Configuration for this swarm's federation identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmIdentityConfig {
    /// Unique identifier for this swarm.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Description of this swarm's purpose.
    pub description: Option<String>,
    /// Contact information for swarm administrators.
    pub contact: Option<String>,
    /// Path to private key file (for key-based auth).
    pub private_key_path: Option<String>,
    /// Path to public key file (for key-based auth).
    pub public_key_path: Option<String>,
    /// Path to TLS certificate (for mTLS).
    pub tls_cert_path: Option<String>,
    /// Path to TLS key (for mTLS).
    pub tls_key_path: Option<String>,
}

impl Default for SwarmIdentityConfig {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "unnamed-swarm".to_string(),
            description: None,
            contact: None,
            private_key_path: None,
            public_key_path: None,
            tls_cert_path: None,
            tls_key_path: None,
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

/// Configuration for the budget-aware scheduling subsystem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetConfig {
    /// Consumed-% threshold that promotes pressure to Caution (default 0.60).
    pub caution_threshold_pct: f64,
    /// Consumed-% threshold that promotes pressure to Warning (default 0.80).
    pub warning_threshold_pct: f64,
    /// Consumed-% threshold that promotes pressure to Critical (default 0.95).
    pub critical_threshold_pct: f64,
    /// Consumed-% below which an opportunity window is signalled (default 0.30).
    pub opportunity_threshold_pct: f64,
    /// Minimum remaining tokens required to declare an opportunity window.
    pub min_opportunity_tokens: u64,
    /// Maximum concurrent agents when pressure is Normal.
    pub max_agents_normal: u32,
    /// Maximum concurrent agents when pressure is Caution.
    pub max_agents_caution: u32,
    /// Maximum concurrent agents when pressure is Warning.
    pub max_agents_warning: u32,
    /// Maximum concurrent agents when pressure is Critical.
    pub max_agents_critical: u32,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            caution_threshold_pct: 0.60,
            warning_threshold_pct: 0.80,
            critical_threshold_pct: 0.95,
            opportunity_threshold_pct: 0.30,
            min_opportunity_tokens: 10_000,
            max_agents_normal: 5,
            max_agents_caution: 4,
            max_agents_warning: 2,
            max_agents_critical: 1,
        }
    }
}

/// Configuration for the external adapter subsystem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdapterConfig {
    /// Whether the adapter subsystem is enabled.
    pub enabled: bool,
    /// Directory containing adapter definitions (relative to project root).
    pub adapters_dir: String,
    /// Default polling interval for ingestion adapters (seconds).
    pub default_poll_interval_secs: u64,
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            adapters_dir: ".abathur/adapters".to_string(),
            default_poll_interval_secs: 300,
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
        if let Ok(val) = std::env::var("ABATHUR_LIMITS_MAX_DEPTH")
            && let Ok(v) = val.parse() { self.limits.max_depth = v; }
        if let Ok(val) = std::env::var("ABATHUR_DATABASE_PATH") {
            self.database.path = val;
        }
        if let Ok(val) = std::env::var("ABATHUR_LOG_LEVEL") {
            self.logging.level = val;
        }
        if let Ok(val) = std::env::var("ABATHUR_DEFAULT_WORKFLOW") {
            self.default_workflow = val;
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

        // Validate each workflow template.
        for wf in &self.workflows {
            if let Err(reason) = wf.validate() {
                return Err(ConfigError::ValidationError {
                    field: format!("workflows.{}", wf.name),
                    reason,
                });
            }
        }

        // Check for duplicate workflow names.
        let mut seen_names = std::collections::HashSet::new();
        for wf in &self.workflows {
            if !seen_names.insert(&wf.name) {
                return Err(ConfigError::ValidationError {
                    field: "workflows".to_string(),
                    reason: format!("duplicate workflow name '{}'", wf.name),
                });
            }
        }

        // Verify default_workflow references a known workflow (user-defined or a built-in).
        let builtin_names = ["code", "analysis", "docs", "review"];
        if !builtin_names.contains(&self.default_workflow.as_str())
            && !self.workflows.iter().any(|wf| wf.name == self.default_workflow)
        {
            return Err(ConfigError::ValidationError {
                field: "default_workflow".to_string(),
                reason: format!(
                    "default workflow '{}' not found in defined workflows or built-in workflows \
                     (built-ins: {})",
                    self.default_workflow,
                    builtin_names.join(", "),
                ),
            });
        }

        Ok(())
    }

    /// Resolve a workflow template by name.
    ///
    /// Checks user-defined workflows first, then falls back to the built-in workflows:
    /// - `"code"` → 4-phase code workflow (research, plan, implement, review)
    /// - `"analysis"` → 3-phase read-only analysis workflow (memory-only output)
    /// - `"docs"` → 3-phase documentation workflow (PR output)
    /// - `"review"` → single-phase code review workflow (memory-only output)
    /// - `"external"` → 5-phase triage-first workflow for adapter-sourced tasks
    ///   (triage, research, plan, implement, review)
    pub fn resolve_workflow(&self, name: &str) -> Option<WorkflowTemplate> {
        // Check user-defined workflows first.
        if let Some(wf) = self.workflows.iter().find(|wf| wf.name == name) {
            return Some(wf.clone());
        }

        // Fall back to built-in workflows.
        match name {
            "code" => Some(WorkflowTemplate::default_code_workflow()),
            "analysis" => Some(WorkflowTemplate::analysis_workflow()),
            "docs" => Some(WorkflowTemplate::docs_workflow()),
            "review" => Some(WorkflowTemplate::review_only_workflow()),
            "external" => Some(WorkflowTemplate::external_workflow()),
            _ => None,
        }
    }

    /// Returns the default workflow template.
    pub fn default_workflow_template(&self) -> WorkflowTemplate {
        self.resolve_workflow(&self.default_workflow)
            .unwrap_or_else(WorkflowTemplate::default_code_workflow)
    }

    /// Returns available workflows as (name, description, phase_count, is_default).
    ///
    /// Lists all built-in workflows followed by user-defined workflows.
    /// A built-in workflow is omitted from the built-in list if the user has
    /// defined a workflow with the same name (user definition takes precedence).
    pub fn available_workflows(&self) -> Vec<(String, String, usize, bool)> {
        let mut workflows = Vec::new();

        // Add each built-in workflow unless shadowed by a user-defined one.
        let builtins: [(&str, fn() -> WorkflowTemplate); 5] = [
            ("code",     WorkflowTemplate::default_code_workflow),
            ("analysis", WorkflowTemplate::analysis_workflow),
            ("docs",     WorkflowTemplate::docs_workflow),
            ("review",   WorkflowTemplate::review_only_workflow),
            ("external", WorkflowTemplate::external_workflow),
        ];
        for (name, constructor) in &builtins {
            if !self.workflows.iter().any(|wf| wf.name == *name) {
                let builtin = constructor();
                workflows.push((
                    builtin.name.clone(),
                    builtin.description.clone(),
                    builtin.phases.len(),
                    self.default_workflow == *name,
                ));
            }
        }

        // Add user-defined workflows.
        for wf in &self.workflows {
            workflows.push((
                wf.name.clone(),
                wf.description.clone(),
                wf.phases.len(),
                self.default_workflow == wf.name,
            ));
        }

        workflows
    }

    pub fn sample_toml() -> String {
        toml::to_string_pretty(&Config::default()).unwrap_or_default()
    }
}

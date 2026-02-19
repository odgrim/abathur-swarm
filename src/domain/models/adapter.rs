//! Adapter domain models.
//!
//! Adapters are plugins that connect the swarm to external systems.
//! They come in two flavors: ingestion adapters (pull work in) and
//! egress adapters (push results out). An adapter can be bidirectional.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The implementation strategy for an adapter.
///
/// Prompt adapters use LLM-based translation (adapter provides a prompt
/// template and the swarm uses an LLM to transform between external
/// and internal formats). Native adapters are compiled Rust code that
/// implements the adapter traits directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterType {
    /// LLM-based adapter using prompt templates for format translation.
    Prompt,
    /// Compiled Rust adapter implementing traits directly.
    Native,
}

impl Default for AdapterType {
    fn default() -> Self {
        Self::Prompt
    }
}

impl AdapterType {
    /// Returns the string representation of this adapter type.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Native => "native",
        }
    }

    /// Parse an adapter type from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "prompt" => Some(Self::Prompt),
            "native" => Some(Self::Native),
            _ => None,
        }
    }
}

/// The data-flow direction an adapter supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterDirection {
    /// Pulls items from an external system into the swarm.
    Ingestion,
    /// Pushes results from the swarm to an external system.
    Egress,
    /// Supports both ingestion and egress.
    Bidirectional,
}

impl Default for AdapterDirection {
    fn default() -> Self {
        Self::Ingestion
    }
}

impl AdapterDirection {
    /// Returns the string representation of this direction.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ingestion => "ingestion",
            Self::Egress => "egress",
            Self::Bidirectional => "bidirectional",
        }
    }

    /// Parse a direction from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ingestion" | "ingest" => Some(Self::Ingestion),
            "egress" => Some(Self::Egress),
            "bidirectional" | "both" => Some(Self::Bidirectional),
            _ => None,
        }
    }

    /// Whether this direction supports ingestion.
    pub fn supports_ingestion(&self) -> bool {
        matches!(self, Self::Ingestion | Self::Bidirectional)
    }

    /// Whether this direction supports egress.
    pub fn supports_egress(&self) -> bool {
        matches!(self, Self::Egress | Self::Bidirectional)
    }
}

/// Capabilities that an adapter can declare.
///
/// Each capability maps to a specific operation the adapter can perform.
/// The manifest declares which capabilities are available so the swarm
/// can route actions to the correct adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterCapability {
    /// Can poll for new or updated items from the external system.
    PollItems,
    /// Can update the status of an external item (e.g., move a Jira ticket).
    UpdateStatus,
    /// Can post comments or notes on an external item.
    PostComment,
    /// Can create new items in the external system.
    CreateItem,
    /// Can attach files or artifacts to an external item.
    AttachArtifact,
    /// Can map external priority values to internal TaskPriority.
    MapPriority,
    /// Can perform custom adapter-specific actions.
    Custom,
}

impl AdapterCapability {
    /// Returns the string representation of this capability.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PollItems => "poll_items",
            Self::UpdateStatus => "update_status",
            Self::PostComment => "post_comment",
            Self::CreateItem => "create_item",
            Self::AttachArtifact => "attach_artifact",
            Self::MapPriority => "map_priority",
            Self::Custom => "custom",
        }
    }

    /// Parse a capability from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "poll_items" => Some(Self::PollItems),
            "update_status" => Some(Self::UpdateStatus),
            "post_comment" => Some(Self::PostComment),
            "create_item" => Some(Self::CreateItem),
            "attach_artifact" => Some(Self::AttachArtifact),
            "map_priority" => Some(Self::MapPriority),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// Manifest describing an adapter's identity, type, and capabilities.
///
/// This is the metadata that an adapter declares (typically deserialized
/// from an `adapter.toml` file). The swarm uses this to discover what
/// adapters are available and what they can do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterManifest {
    /// Unique name for this adapter (e.g., "github-issues", "jira", "linear").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Semantic version string (e.g., "0.1.0").
    pub version: String,
    /// Implementation strategy.
    pub adapter_type: AdapterType,
    /// Data-flow direction.
    pub direction: AdapterDirection,
    /// Declared capabilities.
    pub capabilities: Vec<AdapterCapability>,
    /// Adapter-specific configuration (e.g., API base URL, project key).
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

impl AdapterManifest {
    /// Create a new adapter manifest with required fields.
    pub fn new(
        name: impl Into<String>,
        adapter_type: AdapterType,
        direction: AdapterDirection,
    ) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            version: "0.1.0".to_string(),
            adapter_type,
            direction,
            capabilities: Vec::new(),
            config: HashMap::new(),
        }
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Add a capability.
    pub fn with_capability(mut self, cap: AdapterCapability) -> Self {
        if !self.capabilities.contains(&cap) {
            self.capabilities.push(cap);
        }
        self
    }

    /// Add a config entry.
    pub fn with_config(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.config.insert(key.into(), value);
        self
    }

    /// Check whether this adapter has a specific capability.
    pub fn has_capability(&self, cap: AdapterCapability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Validate the manifest for correctness.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("Adapter name cannot be empty".to_string());
        }
        if self.name.len() > 64 {
            return Err("Adapter name cannot exceed 64 characters".to_string());
        }
        // Name must be a valid identifier: lowercase alphanumeric + hyphens
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(
                "Adapter name must contain only lowercase letters, digits, and hyphens".to_string(),
            );
        }
        if self.version.is_empty() {
            return Err("Adapter version cannot be empty".to_string());
        }
        if self.capabilities.is_empty() {
            return Err("Adapter must declare at least one capability".to_string());
        }
        // Validate direction vs capabilities
        if self.direction.supports_ingestion()
            && !self.has_capability(AdapterCapability::PollItems)
        {
            return Err(
                "Ingestion adapters must declare the PollItems capability".to_string(),
            );
        }
        Ok(())
    }
}

/// An action to execute against an external system via an egress adapter.
///
/// This is a tagged enum: each variant carries the parameters needed
/// for that particular action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EgressAction {
    /// Update the status of an external item.
    UpdateStatus {
        /// The external item's identifier.
        external_id: String,
        /// The new status value (adapter-specific, e.g., "In Progress", "Done").
        new_status: String,
    },
    /// Post a comment on an external item.
    PostComment {
        /// The external item's identifier.
        external_id: String,
        /// The comment body (may contain markdown).
        body: String,
    },
    /// Create a new item in the external system.
    CreateItem {
        /// Title for the new item.
        title: String,
        /// Description body.
        description: String,
        /// Additional fields (adapter-specific).
        #[serde(default)]
        fields: HashMap<String, serde_json::Value>,
    },
    /// Attach an artifact to an external item.
    AttachArtifact {
        /// The external item's identifier.
        external_id: String,
        /// URI of the artifact (e.g., a file path or URL).
        artifact_uri: String,
        /// Optional label for the attachment.
        label: Option<String>,
    },
    /// A custom adapter-specific action.
    Custom {
        /// Action name (adapter-defined).
        action_name: String,
        /// Arbitrary parameters.
        #[serde(default)]
        params: HashMap<String, serde_json::Value>,
    },
}

/// A directive to execute an egress action through a named adapter.
///
/// This is the command object that the swarm sends to the egress
/// subsystem. It names the adapter and the action to perform, and
/// optionally links the action to a specific task for traceability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressDirective {
    /// Name of the adapter to route this action to.
    pub adapter_name: String,
    /// The action to perform.
    pub action: EgressAction,
    /// Optional task ID that triggered this egress (for traceability).
    pub task_id: Option<uuid::Uuid>,
}

impl EgressDirective {
    /// Create a new egress directive.
    pub fn new(adapter_name: impl Into<String>, action: EgressAction) -> Self {
        Self {
            adapter_name: adapter_name.into(),
            action,
            task_id: None,
        }
    }

    /// Associate this directive with a task.
    pub fn with_task_id(mut self, task_id: uuid::Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }
}

/// The result of executing an egress action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressResult {
    /// Whether the action completed successfully.
    pub success: bool,
    /// External system's identifier for the created/updated resource.
    pub external_id: Option<String>,
    /// URL to view the resource in the external system's UI.
    pub external_url: Option<String>,
    /// Error message if the action failed.
    pub error: Option<String>,
}

impl EgressResult {
    /// Create a successful egress result.
    pub fn ok() -> Self {
        Self {
            success: true,
            external_id: None,
            external_url: None,
            error: None,
        }
    }

    /// Create a successful result with an external ID.
    pub fn ok_with_id(external_id: impl Into<String>) -> Self {
        Self {
            success: true,
            external_id: Some(external_id.into()),
            external_url: None,
            error: None,
        }
    }

    /// Create a failed egress result.
    pub fn fail(error: impl Into<String>) -> Self {
        Self {
            success: false,
            external_id: None,
            external_url: None,
            error: Some(error.into()),
        }
    }

    /// Set the external URL.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.external_url = Some(url.into());
        self
    }
}

/// An item ingested from an external system.
///
/// This is the normalized representation of work items pulled in by
/// ingestion adapters. The adapter is responsible for mapping external
/// fields to these common fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionItem {
    /// Identifier in the external system (e.g., "PROJ-123", "issue #42").
    pub external_id: String,
    /// Title/summary of the item.
    pub title: String,
    /// Full description or body text.
    pub description: String,
    /// Suggested priority (mapped by the adapter from external priority).
    pub priority: Option<crate::domain::models::TaskPriority>,
    /// Arbitrary adapter-specific metadata.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// When this item was last updated in the external system.
    pub external_updated_at: Option<DateTime<Utc>>,
}

impl IngestionItem {
    /// Create a new ingestion item with required fields.
    pub fn new(
        external_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            external_id: external_id.into(),
            title: title.into(),
            description: description.into(),
            priority: None,
            metadata: HashMap::new(),
            external_updated_at: None,
        }
    }

    /// Set the priority.
    pub fn with_priority(mut self, priority: crate::domain::models::TaskPriority) -> Self {
        self.priority = Some(priority);
        self
    }

    /// Set the external updated timestamp.
    pub fn with_external_updated_at(mut self, ts: DateTime<Utc>) -> Self {
        self.external_updated_at = Some(ts);
        self
    }

    /// Add a metadata entry.
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_type_round_trip() {
        assert_eq!(AdapterType::from_str("prompt"), Some(AdapterType::Prompt));
        assert_eq!(AdapterType::from_str("native"), Some(AdapterType::Native));
        assert_eq!(AdapterType::from_str("unknown"), None);
        assert_eq!(AdapterType::Prompt.as_str(), "prompt");
        assert_eq!(AdapterType::Native.as_str(), "native");
    }

    #[test]
    fn test_adapter_direction_round_trip() {
        assert_eq!(
            AdapterDirection::from_str("ingestion"),
            Some(AdapterDirection::Ingestion)
        );
        assert_eq!(
            AdapterDirection::from_str("egress"),
            Some(AdapterDirection::Egress)
        );
        assert_eq!(
            AdapterDirection::from_str("bidirectional"),
            Some(AdapterDirection::Bidirectional)
        );
        assert_eq!(
            AdapterDirection::from_str("both"),
            Some(AdapterDirection::Bidirectional)
        );
        assert_eq!(AdapterDirection::from_str("unknown"), None);
    }

    #[test]
    fn test_direction_supports() {
        assert!(AdapterDirection::Ingestion.supports_ingestion());
        assert!(!AdapterDirection::Ingestion.supports_egress());

        assert!(!AdapterDirection::Egress.supports_ingestion());
        assert!(AdapterDirection::Egress.supports_egress());

        assert!(AdapterDirection::Bidirectional.supports_ingestion());
        assert!(AdapterDirection::Bidirectional.supports_egress());
    }

    #[test]
    fn test_adapter_capability_round_trip() {
        let caps = vec![
            (AdapterCapability::PollItems, "poll_items"),
            (AdapterCapability::UpdateStatus, "update_status"),
            (AdapterCapability::PostComment, "post_comment"),
            (AdapterCapability::CreateItem, "create_item"),
            (AdapterCapability::AttachArtifact, "attach_artifact"),
            (AdapterCapability::MapPriority, "map_priority"),
            (AdapterCapability::Custom, "custom"),
        ];
        for (cap, s) in caps {
            assert_eq!(cap.as_str(), s);
            assert_eq!(AdapterCapability::from_str(s), Some(cap));
        }
        assert_eq!(AdapterCapability::from_str("nonexistent"), None);
    }

    #[test]
    fn test_manifest_validation_valid() {
        let manifest = AdapterManifest::new(
            "github-issues",
            AdapterType::Native,
            AdapterDirection::Ingestion,
        )
        .with_description("GitHub Issues adapter")
        .with_capability(AdapterCapability::PollItems);

        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_manifest_validation_empty_name() {
        let manifest = AdapterManifest::new("", AdapterType::Prompt, AdapterDirection::Ingestion)
            .with_capability(AdapterCapability::PollItems);

        assert!(manifest.validate().is_err());
    }

    #[test]
    fn test_manifest_validation_invalid_name_chars() {
        let manifest =
            AdapterManifest::new("My Adapter!", AdapterType::Prompt, AdapterDirection::Ingestion)
                .with_capability(AdapterCapability::PollItems);

        let err = manifest.validate().unwrap_err();
        assert!(err.contains("lowercase"));
    }

    #[test]
    fn test_manifest_validation_no_capabilities() {
        let manifest = AdapterManifest::new(
            "test-adapter",
            AdapterType::Prompt,
            AdapterDirection::Egress,
        );

        let err = manifest.validate().unwrap_err();
        assert!(err.contains("capability"));
    }

    #[test]
    fn test_manifest_validation_ingestion_requires_poll() {
        let manifest = AdapterManifest::new(
            "test-adapter",
            AdapterType::Prompt,
            AdapterDirection::Ingestion,
        )
        .with_capability(AdapterCapability::UpdateStatus);

        let err = manifest.validate().unwrap_err();
        assert!(err.contains("PollItems"));
    }

    #[test]
    fn test_manifest_egress_no_poll_required() {
        let manifest = AdapterManifest::new(
            "test-adapter",
            AdapterType::Native,
            AdapterDirection::Egress,
        )
        .with_capability(AdapterCapability::UpdateStatus);

        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_egress_action_serialization() {
        let action = EgressAction::UpdateStatus {
            external_id: "PROJ-123".to_string(),
            new_status: "Done".to_string(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"action\":\"update_status\""));

        let deserialized: EgressAction = serde_json::from_str(&json).unwrap();
        match deserialized {
            EgressAction::UpdateStatus {
                external_id,
                new_status,
            } => {
                assert_eq!(external_id, "PROJ-123");
                assert_eq!(new_status, "Done");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_egress_action_post_comment() {
        let action = EgressAction::PostComment {
            external_id: "ISSUE-42".to_string(),
            body: "Task completed successfully.".to_string(),
        };
        let json = serde_json::to_string(&action).unwrap();
        let deserialized: EgressAction = serde_json::from_str(&json).unwrap();
        match deserialized {
            EgressAction::PostComment { external_id, body } => {
                assert_eq!(external_id, "ISSUE-42");
                assert_eq!(body, "Task completed successfully.");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_egress_action_create_item() {
        let mut fields = HashMap::new();
        fields.insert(
            "labels".to_string(),
            serde_json::json!(["bug", "critical"]),
        );
        let action = EgressAction::CreateItem {
            title: "New bug".to_string(),
            description: "Something is broken".to_string(),
            fields,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"action\":\"create_item\""));
    }

    #[test]
    fn test_egress_directive_builder() {
        let task_id = uuid::Uuid::new_v4();
        let directive = EgressDirective::new(
            "jira",
            EgressAction::UpdateStatus {
                external_id: "PROJ-1".to_string(),
                new_status: "In Progress".to_string(),
            },
        )
        .with_task_id(task_id);

        assert_eq!(directive.adapter_name, "jira");
        assert_eq!(directive.task_id, Some(task_id));
    }

    #[test]
    fn test_egress_result_ok() {
        let result = EgressResult::ok_with_id("PROJ-123").with_url("https://jira.example.com/PROJ-123");
        assert!(result.success);
        assert_eq!(result.external_id.as_deref(), Some("PROJ-123"));
        assert!(result.external_url.is_some());
        assert!(result.error.is_none());
    }

    #[test]
    fn test_egress_result_fail() {
        let result = EgressResult::fail("API rate limited");
        assert!(!result.success);
        assert!(result.external_id.is_none());
        assert_eq!(result.error.as_deref(), Some("API rate limited"));
    }

    #[test]
    fn test_ingestion_item_builder() {
        let item = IngestionItem::new("GH-42", "Fix login bug", "The login page crashes on submit")
            .with_priority(crate::domain::models::TaskPriority::High)
            .with_metadata("labels".to_string(), serde_json::json!(["bug"]))
            .with_external_updated_at(Utc::now());

        assert_eq!(item.external_id, "GH-42");
        assert_eq!(item.title, "Fix login bug");
        assert_eq!(
            item.priority,
            Some(crate::domain::models::TaskPriority::High)
        );
        assert!(item.metadata.contains_key("labels"));
        assert!(item.external_updated_at.is_some());
    }

    #[test]
    fn test_manifest_has_capability() {
        let manifest = AdapterManifest::new(
            "test",
            AdapterType::Native,
            AdapterDirection::Bidirectional,
        )
        .with_capability(AdapterCapability::PollItems)
        .with_capability(AdapterCapability::UpdateStatus);

        assert!(manifest.has_capability(AdapterCapability::PollItems));
        assert!(manifest.has_capability(AdapterCapability::UpdateStatus));
        assert!(!manifest.has_capability(AdapterCapability::PostComment));
    }

    #[test]
    fn test_manifest_dedup_capabilities() {
        let manifest = AdapterManifest::new(
            "test",
            AdapterType::Native,
            AdapterDirection::Egress,
        )
        .with_capability(AdapterCapability::UpdateStatus)
        .with_capability(AdapterCapability::UpdateStatus);

        assert_eq!(manifest.capabilities.len(), 1);
    }

    #[test]
    fn test_adapter_type_default() {
        assert_eq!(AdapterType::default(), AdapterType::Prompt);
    }

    #[test]
    fn test_adapter_direction_default() {
        assert_eq!(AdapterDirection::default(), AdapterDirection::Ingestion);
    }

    #[test]
    fn test_manifest_config() {
        let manifest = AdapterManifest::new(
            "jira",
            AdapterType::Native,
            AdapterDirection::Bidirectional,
        )
        .with_capability(AdapterCapability::PollItems)
        .with_config("base_url", serde_json::json!("https://jira.example.com"))
        .with_config("project_key", serde_json::json!("PROJ"));

        assert_eq!(
            manifest.config.get("base_url"),
            Some(&serde_json::json!("https://jira.example.com"))
        );
        assert_eq!(
            manifest.config.get("project_key"),
            Some(&serde_json::json!("PROJ"))
        );
    }

    #[test]
    fn test_egress_action_custom() {
        let mut params = HashMap::new();
        params.insert("webhook_url".to_string(), serde_json::json!("https://hooks.example.com/notify"));
        let action = EgressAction::Custom {
            action_name: "notify_slack".to_string(),
            params,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"action\":\"custom\""));
        assert!(json.contains("notify_slack"));
    }
}

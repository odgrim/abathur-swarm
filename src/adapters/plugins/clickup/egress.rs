//! ClickUp egress adapter.
//!
//! Executes egress actions against the ClickUp API, mapping each
//! [`EgressAction`] variant to the corresponding API call.

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::adapter::{
    AdapterManifest, EgressAction, EgressResult,
};
use crate::domain::ports::adapter::EgressAdapter;

use super::client::ClickUpClient;

/// Adapter that pushes actions to ClickUp (status updates, comments, task creation).
///
/// Configuration is read from the [`AdapterManifest::config`] map:
/// - `list_id` (required for `CreateItem`): the default ClickUp list for new tasks.
#[derive(Debug)]
pub struct ClickUpEgressAdapter {
    /// The adapter manifest describing capabilities and config.
    manifest: AdapterManifest,
    /// Shared ClickUp HTTP client.
    client: Arc<ClickUpClient>,
}

impl ClickUpEgressAdapter {
    /// Create a new egress adapter.
    pub fn new(manifest: AdapterManifest, client: Arc<ClickUpClient>) -> Self {
        Self { manifest, client }
    }

    /// Read the `list_id` from the manifest config.
    fn list_id(&self) -> Option<&str> {
        self.manifest
            .config
            .get("list_id")
            .and_then(|v| v.as_str())
    }
}

#[async_trait]
impl EgressAdapter for ClickUpEgressAdapter {
    fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    async fn execute(&self, action: &EgressAction) -> DomainResult<EgressResult> {
        match action {
            EgressAction::UpdateStatus {
                external_id,
                new_status,
            } => {
                tracing::info!(
                    task_id = %external_id,
                    status = %new_status,
                    "ClickUp: updating task status"
                );
                self.client
                    .update_task_status(external_id, new_status)
                    .await?;
                Ok(EgressResult::ok_with_id(external_id))
            }

            EgressAction::PostComment { external_id, body } => {
                tracing::info!(
                    task_id = %external_id,
                    body_len = body.len(),
                    "ClickUp: posting comment"
                );
                self.client.post_comment(external_id, body).await?;
                Ok(EgressResult::ok_with_id(external_id))
            }

            EgressAction::CreateItem {
                title,
                description,
                fields,
            } => {
                // Use list_id from fields override, or fall back to manifest config.
                let list_id = fields
                    .get("list_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| self.list_id())
                    .ok_or_else(|| {
                        DomainError::ValidationFailed(
                            "ClickUp CreateItem requires 'list_id' in fields or adapter config"
                                .to_string(),
                        )
                    })?;

                tracing::info!(
                    list_id = %list_id,
                    title = %title,
                    "ClickUp: creating task"
                );

                let resp = self.client.create_task(list_id, title, description).await?;

                let mut result = EgressResult::ok_with_id(&resp.id);
                if let Some(ref url) = resp.url {
                    result = result.with_url(url);
                }
                Ok(result)
            }

            EgressAction::AttachArtifact { external_id, .. } => {
                tracing::debug!(
                    task_id = %external_id,
                    "ClickUp adapter does not support AttachArtifact"
                );
                Ok(EgressResult::fail(
                    "AttachArtifact is not a supported operation for the ClickUp adapter. \
                     Use PostComment to share artifact details instead.",
                ))
            }

            EgressAction::Custom {
                action_name,
                ..
            } => {
                tracing::warn!(
                    action = %action_name,
                    "ClickUp: unknown custom action"
                );
                Ok(EgressResult::fail(format!(
                    "Custom action '{action_name}' is not supported by the ClickUp adapter"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::adapter::{
        AdapterCapability, AdapterDirection, AdapterType,
    };

    fn test_manifest() -> AdapterManifest {
        AdapterManifest::new("clickup", AdapterType::Native, AdapterDirection::Bidirectional)
            .with_capability(AdapterCapability::PollItems)
            .with_capability(AdapterCapability::UpdateStatus)
            .with_capability(AdapterCapability::PostComment)
            .with_capability(AdapterCapability::CreateItem)
            .with_config("list_id", serde_json::json!("99999"))
    }

    #[test]
    fn test_egress_adapter_manifest() {
        let manifest = test_manifest();
        let client = Arc::new(ClickUpClient::new("test-key".to_string()));
        let adapter = ClickUpEgressAdapter::new(manifest.clone(), client);

        assert_eq!(adapter.manifest().name, "clickup");
        assert!(adapter.manifest().has_capability(AdapterCapability::UpdateStatus));
    }

    #[test]
    fn test_list_id_from_config() {
        let manifest = test_manifest();
        let client = Arc::new(ClickUpClient::new("test-key".to_string()));
        let adapter = ClickUpEgressAdapter::new(manifest, client);
        assert_eq!(adapter.list_id(), Some("99999"));
    }

    #[test]
    fn test_list_id_absent() {
        let manifest =
            AdapterManifest::new("clickup", AdapterType::Native, AdapterDirection::Egress)
                .with_capability(AdapterCapability::UpdateStatus);
        let client = Arc::new(ClickUpClient::new("test-key".to_string()));
        let adapter = ClickUpEgressAdapter::new(manifest, client);
        assert!(adapter.list_id().is_none());
    }
}

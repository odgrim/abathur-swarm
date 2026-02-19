//! ClickUp ingestion adapter.
//!
//! Polls a ClickUp list for tasks and maps them to [`IngestionItem`]s.
//! Supports incremental polling via `date_updated_gt` and optional
//! tag-based filtering.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::errors::DomainResult;
use crate::domain::models::adapter::{AdapterManifest, IngestionItem};
use crate::domain::models::TaskPriority;
use crate::domain::ports::adapter::IngestionAdapter;

use super::client::ClickUpClient;
use super::models::ClickUpTask;

/// Adapter that ingests tasks from a ClickUp list.
///
/// Configuration is read from the [`AdapterManifest::config`] map:
/// - `list_id` (required): the ClickUp list to poll.
/// - `filter_tag` (optional): only return tasks with this tag.
#[derive(Debug)]
pub struct ClickUpIngestionAdapter {
    /// The adapter manifest describing capabilities and config.
    manifest: AdapterManifest,
    /// Shared ClickUp HTTP client.
    client: Arc<ClickUpClient>,
}

impl ClickUpIngestionAdapter {
    /// Create a new ingestion adapter.
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

    /// Read the optional `filter_tag` from the manifest config.
    fn filter_tag(&self) -> Option<&str> {
        self.manifest
            .config
            .get("filter_tag")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
    }

    /// Map a ClickUp priority ID to a [`TaskPriority`].
    ///
    /// ClickUp uses: 1 = urgent, 2 = high, 3 = normal, 4 = low.
    fn map_priority(clickup_priority: &super::models::ClickUpPriority) -> TaskPriority {
        match clickup_priority.id.as_str() {
            "1" => TaskPriority::Critical,
            "2" => TaskPriority::High,
            "3" => TaskPriority::Normal,
            "4" => TaskPriority::Low,
            _ => TaskPriority::Normal,
        }
    }

    /// Convert a [`ClickUpTask`] to an [`IngestionItem`].
    fn to_ingestion_item(task: &ClickUpTask) -> IngestionItem {
        let description = task.description.clone().unwrap_or_default();

        let mut item = IngestionItem::new(&task.id, &task.name, description);

        if let Some(ref priority) = task.priority {
            item = item.with_priority(Self::map_priority(priority));
        }

        // Store the ClickUp status as metadata.
        item = item.with_metadata(
            "clickup_status",
            serde_json::json!(task.status.status),
        );

        if let Some(ref url) = task.url {
            item = item.with_metadata("clickup_url", serde_json::json!(url));
        }

        // Store tags as metadata.
        if !task.tags.is_empty() {
            let tag_names: Vec<&str> = task.tags.iter().map(|t| t.name.as_str()).collect();
            item = item.with_metadata("clickup_tags", serde_json::json!(tag_names));
        }

        // Parse date_updated (Unix ms) to DateTime<Utc>.
        if let Some(ref ts_str) = task.date_updated {
            if let Ok(ts_ms) = ts_str.parse::<i64>() {
                if let Some(dt) = DateTime::from_timestamp_millis(ts_ms) {
                    item = item.with_external_updated_at(dt);
                }
            }
        }

        item
    }
}

#[async_trait]
impl IngestionAdapter for ClickUpIngestionAdapter {
    fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    async fn poll(&self, last_poll: Option<DateTime<Utc>>) -> DomainResult<Vec<IngestionItem>> {
        let list_id = self.list_id().ok_or_else(|| {
            crate::domain::errors::DomainError::ValidationFailed(
                "ClickUp adapter config missing required 'list_id'".to_string(),
            )
        })?;

        // Convert last_poll to Unix milliseconds for the ClickUp API.
        let updated_after_ms = last_poll.map(|dt| dt.timestamp_millis());

        tracing::info!(
            list_id = list_id,
            updated_after_ms = ?updated_after_ms,
            "Polling ClickUp for tasks"
        );

        let response = self.client.get_tasks(list_id, updated_after_ms).await?;

        let filter_tag = self.filter_tag();

        let items: Vec<IngestionItem> = response
            .tasks
            .iter()
            .filter(|task| {
                // If a filter_tag is configured, only include tasks with that tag.
                if let Some(tag) = filter_tag {
                    task.tags.iter().any(|t| t.name == tag)
                } else {
                    true
                }
            })
            .map(Self::to_ingestion_item)
            .collect();

        tracing::info!(
            count = items.len(),
            total_fetched = response.tasks.len(),
            "ClickUp ingestion poll complete"
        );

        Ok(items)
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
            .with_config("list_id", serde_json::json!("12345"))
    }

    fn make_clickup_task(id: &str, name: &str, priority_id: Option<&str>) -> ClickUpTask {
        use super::super::models::*;
        ClickUpTask {
            id: id.to_string(),
            name: name.to_string(),
            description: Some("A test task".to_string()),
            status: ClickUpStatus {
                status: "open".to_string(),
                status_type: Some("open".to_string()),
            },
            priority: priority_id.map(|pid| ClickUpPriority {
                id: pid.to_string(),
                priority: "high".to_string(),
            }),
            date_updated: Some("1700000000000".to_string()),
            url: Some("https://app.clickup.com/t/abc".to_string()),
            tags: vec![ClickUpTag {
                name: "abathur".to_string(),
            }],
            list: Some(ClickUpListRef {
                id: "list1".to_string(),
                name: Some("Sprint 1".to_string()),
            }),
        }
    }

    #[test]
    fn test_priority_mapping() {
        use super::super::models::ClickUpPriority;
        let urgent = ClickUpPriority { id: "1".to_string(), priority: "urgent".to_string() };
        let high = ClickUpPriority { id: "2".to_string(), priority: "high".to_string() };
        let normal = ClickUpPriority { id: "3".to_string(), priority: "normal".to_string() };
        let low = ClickUpPriority { id: "4".to_string(), priority: "low".to_string() };
        let unknown = ClickUpPriority { id: "99".to_string(), priority: "???".to_string() };

        assert_eq!(ClickUpIngestionAdapter::map_priority(&urgent), TaskPriority::Critical);
        assert_eq!(ClickUpIngestionAdapter::map_priority(&high), TaskPriority::High);
        assert_eq!(ClickUpIngestionAdapter::map_priority(&normal), TaskPriority::Normal);
        assert_eq!(ClickUpIngestionAdapter::map_priority(&low), TaskPriority::Low);
        assert_eq!(ClickUpIngestionAdapter::map_priority(&unknown), TaskPriority::Normal);
    }

    #[test]
    fn test_to_ingestion_item_full() {
        let task = make_clickup_task("t1", "Fix bug", Some("2"));
        let item = ClickUpIngestionAdapter::to_ingestion_item(&task);

        assert_eq!(item.external_id, "t1");
        assert_eq!(item.title, "Fix bug");
        assert_eq!(item.description, "A test task");
        assert_eq!(item.priority, Some(TaskPriority::High));
        assert!(item.metadata.contains_key("clickup_status"));
        assert!(item.metadata.contains_key("clickup_url"));
        assert!(item.metadata.contains_key("clickup_tags"));
        assert!(item.external_updated_at.is_some());
    }

    #[test]
    fn test_to_ingestion_item_no_priority() {
        let task = make_clickup_task("t2", "No priority task", None);
        let item = ClickUpIngestionAdapter::to_ingestion_item(&task);

        assert_eq!(item.external_id, "t2");
        assert!(item.priority.is_none());
    }

    #[test]
    fn test_list_id_from_config() {
        let manifest = test_manifest();
        let client = Arc::new(ClickUpClient::new("test-key".to_string()));
        let adapter = ClickUpIngestionAdapter::new(manifest, client);
        assert_eq!(adapter.list_id(), Some("12345"));
    }

    #[test]
    fn test_filter_tag_from_config() {
        let manifest = test_manifest()
            .with_config("filter_tag", serde_json::json!("abathur"));
        let client = Arc::new(ClickUpClient::new("test-key".to_string()));
        let adapter = ClickUpIngestionAdapter::new(manifest, client);
        assert_eq!(adapter.filter_tag(), Some("abathur"));
    }

    #[test]
    fn test_filter_tag_absent() {
        let manifest = test_manifest();
        let client = Arc::new(ClickUpClient::new("test-key".to_string()));
        let adapter = ClickUpIngestionAdapter::new(manifest, client);
        assert!(adapter.filter_tag().is_none());
    }

    #[test]
    fn test_filter_tag_empty_string_treated_as_absent() {
        let manifest = test_manifest()
            .with_config("filter_tag", serde_json::json!(""));
        let client = Arc::new(ClickUpClient::new("test-key".to_string()));
        let adapter = ClickUpIngestionAdapter::new(manifest, client);
        assert!(adapter.filter_tag().is_none());
    }
}

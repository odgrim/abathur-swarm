//! Built-in reactive event handler.
//!
//! All handlers are **idempotent** — safe to run even if the poll loop already
//! handled the same state change. They check current state before acting.

#![allow(unused_imports)]

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::domain::errors::DomainError;
use crate::domain::models::adapter::IngestionItemKind;
use crate::domain::models::convergence::{AmendmentSource, SpecificationAmendment};
use crate::domain::models::task_schedule::*;
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::{Goal, HumanEscalationEvent, Task, TaskSource, TaskStatus};
use crate::domain::ports::{
    GoalRepository, MemoryRepository, TaskRepository, TaskScheduleRepository, TrajectoryRepository,
    WorktreeRepository,
};
#[cfg(test)]
use crate::services::event_bus::ConvergenceTerminatedPayload;
use crate::services::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, HumanEscalationPayload,
    SequenceNumber, SwarmStatsPayload, TaskResultPayload, UnifiedEvent,
};
use crate::services::event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, HandlerContext, HandlerId, HandlerMetadata,
    HandlerPriority, Reaction,
};
use crate::services::event_store::EventStore;
use crate::services::goal_context_service::GoalContextService;
use crate::services::memory_service::MemoryService;
use crate::services::swarm_orchestrator::SwarmStats;
use crate::services::task_service::TaskService;

use super::{try_update_task, update_with_retry};

// ============================================================================
// AdapterLifecycleSyncHandler (Adapter integration)
// ============================================================================

/// Parses an idempotency key of the form `"adapter:{name}:{external_id}"`.
///
/// Returns `Some((adapter_name, external_id))` on success, `None` otherwise.
/// Uses `splitn(3, ':')` so that colons in the external ID are preserved.
fn parse_idempotency_key(key: &str) -> Option<(&str, &str)> {
    let mut parts = key.splitn(3, ':');
    let prefix = parts.next()?;
    if prefix != "adapter" {
        return None;
    }
    let adapter_name = parts.next()?;
    let external_id = parts.next()?;
    if adapter_name.is_empty() || external_id.is_empty() {
        return None;
    }
    Some((adapter_name, external_id))
}

/// Reads a status string from a manifest's config map, or falls back to a default.
///
/// Looks up `config_key` in `manifest.config`. If the value is a JSON string,
/// returns it; otherwise returns `default`.
fn get_status_string(
    manifest: Option<&crate::domain::models::adapter::AdapterManifest>,
    config_key: &str,
    default: &str,
) -> String {
    if let Some(m) = manifest
        && let Some(val) = m.config.get(config_key)
        && let Some(s) = val.as_str()
    {
        return s.to_string();
    }
    default.to_string()
}

/// Synchronizes task lifecycle state changes back to external systems.
///
/// When a task ingested from an external adapter transitions to Running
/// (claimed), Complete, or Failed, this handler pushes a status update
/// back to the originating system via the registered egress adapter.
pub struct AdapterLifecycleSyncHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
}

impl<T: TaskRepository> AdapterLifecycleSyncHandler<T> {
    /// Create a new adapter lifecycle sync handler.
    pub fn new(
        task_repo: Arc<T>,
        adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
    ) -> Self {
        Self {
            task_repo,
            adapter_registry,
        }
    }

    /// Shared handler logic for a lifecycle transition event.
    ///
    /// Looks up the task, validates it came from an egress-capable adapter,
    /// resolves the status string from the manifest config, and fires an
    /// `UpdateStatus` egress action against the adapter.
    async fn handle_lifecycle(
        &self,
        task_id: uuid::Uuid,
        config_key: &str,
        default_status: &str,
    ) -> Result<Reaction, String> {
        // Look up the task.
        let task = match self.task_repo.get(task_id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                tracing::debug!(
                    task_id = %task_id,
                    "Task not found for lifecycle sync, skipping"
                );
                return Ok(Reaction::None);
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id,
                    error = %e,
                    "Failed to fetch task for lifecycle sync"
                );
                return Ok(Reaction::None);
            }
        };

        // Only act on adapter-sourced tasks.
        let adapter_name = match &task.source {
            TaskSource::Adapter(name) => name.clone(),
            _ => return Ok(Reaction::None),
        };

        // Parse the idempotency key to extract the external_id.
        let idem_key = match &task.idempotency_key {
            Some(k) => k.clone(),
            None => {
                tracing::debug!(
                    task_id = %task_id,
                    adapter = adapter_name,
                    "Task has no idempotency key, skipping lifecycle sync"
                );
                return Ok(Reaction::None);
            }
        };

        let (key_adapter_name, external_id) = match parse_idempotency_key(&idem_key) {
            Some(parsed) => parsed,
            None => {
                tracing::debug!(
                    task_id = %task_id,
                    key = idem_key,
                    "Idempotency key does not match adapter format, skipping"
                );
                return Ok(Reaction::None);
            }
        };

        // Verify the adapter name in the key matches task.source.
        if key_adapter_name != adapter_name.as_str() {
            tracing::debug!(
                task_id = %task_id,
                source_adapter = adapter_name,
                key_adapter = key_adapter_name,
                "Adapter name mismatch between source and idempotency key, skipping"
            );
            return Ok(Reaction::None);
        }

        // Look up the egress adapter.
        let adapter = match self.adapter_registry.get_egress(&adapter_name) {
            Some(a) => a,
            None => {
                tracing::debug!(
                    adapter = adapter_name,
                    task_id = %task_id,
                    "No egress adapter registered for lifecycle sync, skipping"
                );
                return Ok(Reaction::None);
            }
        };

        // Determine the status string from manifest config or default.
        let manifest = self.adapter_registry.get_manifest(&adapter_name);
        let new_status = get_status_string(manifest, config_key, default_status);

        // Allow adapters to opt out of a specific lifecycle transition by
        // setting the status value to "skip". This is useful when the external
        // system manages state through its own mechanism (e.g. GitHub issues
        // closed by PR merge) and the lifecycle sync should leave the item
        // untouched for that event.
        if new_status == "skip" {
            tracing::debug!(
                adapter = adapter_name,
                task_id = %task_id,
                config_key = config_key,
                "Lifecycle sync skipped (status = \"skip\")"
            );
            return Ok(Reaction::None);
        }

        let external_id = external_id.to_string();
        let action_name = format!("UpdateStatus({})", new_status);

        // Execute the egress action.
        let action = crate::domain::models::adapter::EgressAction::UpdateStatus {
            external_id: external_id.clone(),
            new_status,
        };

        match adapter.execute(&action).await {
            Ok(result) => {
                tracing::info!(
                    adapter = adapter_name,
                    task_id = %task_id,
                    external_id = external_id,
                    success = result.success,
                    "Lifecycle sync egress completed"
                );
                let event = crate::services::event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressCompleted {
                        adapter_name: adapter_name.clone(),
                        task_id,
                        action: action_name,
                        success: result.success,
                    },
                );
                Ok(Reaction::EmitEvents(vec![event]))
            }
            Err(e) => {
                tracing::warn!(
                    adapter = adapter_name,
                    task_id = %task_id,
                    external_id = external_id,
                    error = %e,
                    "Lifecycle sync egress failed"
                );
                let event = crate::services::event_factory::make_event(
                    EventSeverity::Warning,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressFailed {
                        adapter_name: adapter_name.clone(),
                        task_id: Some(task_id),
                        error: e.to_string(),
                    },
                );
                Ok(Reaction::EmitEvents(vec![event]))
            }
        }
    }

    /// Handle a gate rejection for an adapter-sourced task.
    ///
    /// Updates the external item's status via `status_rejected` and optionally
    /// posts a comment with the rejection reason when `comment_on_rejection`
    /// is set to `"true"` in the adapter manifest config.
    async fn handle_rejection(
        &self,
        task_id: uuid::Uuid,
        phase_name: &str,
        reason: &str,
    ) -> Result<Reaction, String> {
        // Look up the task.
        let task = match self.task_repo.get(task_id).await {
            Ok(Some(t)) => t,
            _ => return Ok(Reaction::None),
        };

        // Only act on adapter-sourced tasks.
        let adapter_name = match &task.source {
            TaskSource::Adapter(name) => name.clone(),
            _ => return Ok(Reaction::None),
        };

        // Parse external_id from idempotency key.
        let external_id = match task
            .idempotency_key
            .as_deref()
            .and_then(parse_idempotency_key)
        {
            Some((_, eid)) => eid.to_string(),
            None => return Ok(Reaction::None),
        };

        // Look up the egress adapter.
        let adapter = match self.adapter_registry.get_egress(&adapter_name) {
            Some(a) => a,
            None => return Ok(Reaction::None),
        };

        let manifest = self.adapter_registry.get_manifest(&adapter_name);
        let mut events = Vec::new();

        // --- Status update via status_rejected ---
        let new_status = get_status_string(manifest, "status_rejected", "skip");
        if new_status != "skip" {
            let action = crate::domain::models::adapter::EgressAction::UpdateStatus {
                external_id: external_id.clone(),
                new_status,
            };
            match adapter.execute(&action).await {
                Ok(result) => {
                    tracing::info!(
                        adapter = %adapter_name,
                        task_id = %task_id,
                        external_id = %external_id,
                        success = result.success,
                        "Rejection status sync completed"
                    );
                    events.push(crate::services::event_factory::make_event(
                        EventSeverity::Info,
                        EventCategory::Adapter,
                        None,
                        Some(task_id),
                        EventPayload::AdapterEgressCompleted {
                            adapter_name: adapter_name.clone(),
                            task_id,
                            action: "UpdateStatus(rejected)".to_string(),
                            success: result.success,
                        },
                    ));
                }
                Err(e) => {
                    tracing::warn!(
                        adapter = %adapter_name,
                        task_id = %task_id,
                        error = %e,
                        "Rejection status sync failed"
                    );
                    events.push(crate::services::event_factory::make_event(
                        EventSeverity::Warning,
                        EventCategory::Adapter,
                        None,
                        Some(task_id),
                        EventPayload::AdapterEgressFailed {
                            adapter_name: adapter_name.clone(),
                            task_id: Some(task_id),
                            error: e.to_string(),
                        },
                    ));
                }
            }
        }

        // --- Optional rejection comment ---
        let comment_enabled = manifest
            .and_then(|m| m.config.get("comment_on_rejection"))
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if comment_enabled {
            let body = format!("**Rejected during {phase_name}**\n\n{reason}");
            let action = crate::domain::models::adapter::EgressAction::PostComment {
                external_id: external_id.clone(),
                body,
            };
            match adapter.execute(&action).await {
                Ok(result) => {
                    tracing::info!(
                        adapter = %adapter_name,
                        task_id = %task_id,
                        external_id = %external_id,
                        success = result.success,
                        "Rejection comment posted"
                    );
                    events.push(crate::services::event_factory::make_event(
                        EventSeverity::Info,
                        EventCategory::Adapter,
                        None,
                        Some(task_id),
                        EventPayload::AdapterEgressCompleted {
                            adapter_name: adapter_name.clone(),
                            task_id,
                            action: "PostComment(rejection)".to_string(),
                            success: result.success,
                        },
                    ));
                }
                Err(e) => {
                    tracing::warn!(
                        adapter = %adapter_name,
                        task_id = %task_id,
                        error = %e,
                        "Rejection comment failed"
                    );
                    events.push(crate::services::event_factory::make_event(
                        EventSeverity::Warning,
                        EventCategory::Adapter,
                        None,
                        Some(task_id),
                        EventPayload::AdapterEgressFailed {
                            adapter_name: adapter_name.clone(),
                            task_id: Some(task_id),
                            error: e.to_string(),
                        },
                    ));
                }
            }
        }

        if events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(events))
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for AdapterLifecycleSyncHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "AdapterLifecycleSyncHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![
                    EventCategory::Task,
                    EventCategory::Adapter,
                    EventCategory::Workflow,
                ])
                .payload_types(vec![
                    "TaskClaimed".to_string(),
                    "TaskCompleted".to_string(),
                    "TaskFailed".to_string(),
                    "AdapterTaskIngested".to_string(),
                    "WorkflowGateRejected".to_string(),
                ]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        match &event.payload {
            EventPayload::TaskClaimed { task_id, .. } => {
                self.handle_lifecycle(*task_id, "status_in_progress", "skip")
                    .await
            }
            EventPayload::TaskCompleted { task_id, .. } => {
                self.handle_lifecycle(*task_id, "status_done", "skip").await
            }
            EventPayload::TaskFailed { task_id, .. } => {
                // If this failure was caused by a gate rejection, skip —
                // the WorkflowGateRejected handler owns the egress for
                // rejections and uses the separate status_rejected config.
                if let Ok(Some(task)) = self.task_repo.get(*task_id).await
                    && let Some(crate::domain::models::workflow_state::WorkflowState::Rejected {
                        ..
                    }) =
                        crate::services::workflow_engine::WorkflowEngine::<T>::read_state_from_task(
                            &task,
                        )
                {
                    return Ok(Reaction::None);
                }
                self.handle_lifecycle(*task_id, "status_failed", "skip")
                    .await
            }
            EventPayload::AdapterTaskIngested { task_id, .. } => {
                self.handle_lifecycle(*task_id, "status_pending", "skip")
                    .await
            }
            EventPayload::WorkflowGateRejected {
                task_id,
                phase_name,
                reason,
                ..
            } => self.handle_rejection(*task_id, phase_name, reason).await,
            _ => Ok(Reaction::None),
        }
    }
}

#[cfg(test)]
mod adapter_lifecycle_sync_tests {
    use super::*;

    #[test]
    fn test_parse_idempotency_key_valid() {
        let result = parse_idempotency_key("adapter:clickup:abc123");
        assert_eq!(result, Some(("clickup", "abc123")));
    }

    #[test]
    fn test_parse_idempotency_key_colon_in_external_id() {
        // Colons in the external ID must be preserved via splitn(3, ':')
        let result = parse_idempotency_key("adapter:jira:PROJ:123");
        assert_eq!(result, Some(("jira", "PROJ:123")));
    }

    #[test]
    fn test_parse_idempotency_key_wrong_prefix() {
        let result = parse_idempotency_key("schedule:jira:abc");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_idempotency_key_missing_external_id() {
        // Only two parts — no external_id segment
        assert!(parse_idempotency_key("adapter:clickup").is_none());
    }

    #[test]
    fn test_parse_idempotency_key_empty_adapter_name() {
        assert!(parse_idempotency_key("adapter::external123").is_none());
    }

    #[test]
    fn test_parse_idempotency_key_empty_external_id() {
        assert!(parse_idempotency_key("adapter:clickup:").is_none());
    }

    #[test]
    fn test_parse_idempotency_key_no_colon() {
        assert!(parse_idempotency_key("notadapter").is_none());
    }

    #[test]
    fn test_get_status_string_from_manifest_config() {
        use crate::domain::models::adapter::{AdapterDirection, AdapterManifest, AdapterType};

        let manifest = AdapterManifest::new(
            "clickup",
            AdapterType::Native,
            AdapterDirection::Bidirectional,
        )
        .with_config(
            "status_done",
            serde_json::Value::String("complete".to_string()),
        );

        let result = get_status_string(Some(&manifest), "status_done", "done");
        assert_eq!(result, "complete");
    }

    #[test]
    fn test_get_status_string_fallback_no_manifest() {
        let result = get_status_string(None, "status_done", "done");
        assert_eq!(result, "done");
    }

    #[test]
    fn test_get_status_string_missing_key_uses_default() {
        use crate::domain::models::adapter::{AdapterDirection, AdapterManifest, AdapterType};

        // Manifest with no config entries
        let manifest = AdapterManifest::new(
            "clickup",
            AdapterType::Native,
            AdapterDirection::Bidirectional,
        );

        let result = get_status_string(Some(&manifest), "status_in_progress", "in progress");
        assert_eq!(result, "in progress");
    }

    #[test]
    fn test_get_status_string_non_string_value_uses_default() {
        use crate::domain::models::adapter::{AdapterDirection, AdapterManifest, AdapterType};

        // Config value is a number, not a string
        let manifest = AdapterManifest::new(
            "clickup",
            AdapterType::Native,
            AdapterDirection::Bidirectional,
        )
        .with_config("status_done", serde_json::json!(42));

        let result = get_status_string(Some(&manifest), "status_done", "done");
        assert_eq!(result, "done");
    }
}

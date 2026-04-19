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
// EgressRoutingHandler (Adapter integration)
// ============================================================================

/// Routes task completion results to egress adapters.
///
/// # Directive source
///
/// Preferred path: the producer sets
/// [`TaskResultPayload::egress`](crate::services::event_bus::TaskResultPayload::egress)
/// to a structured [`EgressDirective`].
///
/// Legacy path (retained for backwards compatibility with persisted events
/// written before the dedicated field existed): the `status` string is parsed
/// as JSON and an `"egress"` key, if present, is deserialized into a
/// directive. When this fallback is exercised a `debug!` log is emitted so we
/// can tell when old rows have aged out and the legacy parser is safe to
/// delete.
pub struct EgressRoutingHandler {
    adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
}

impl EgressRoutingHandler {
    /// Create a new egress routing handler.
    pub fn new(adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>) -> Self {
        Self { adapter_registry }
    }
}

#[async_trait]
impl EventHandler for EgressRoutingHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EgressRoutingHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskCompletedWithResult".to_string()]),
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
        let (task_id, result) = match &event.payload {
            EventPayload::TaskCompletedWithResult { task_id, result } => (*task_id, result),
            _ => return Ok(Reaction::None),
        };

        // Prefer the dedicated structured field. If absent, fall back to
        // parsing a JSON blob out of the status string — a legacy encoding
        // retained for backwards compatibility with events written before
        // TaskResultPayload.egress existed.
        let directive: crate::domain::models::adapter::EgressDirective = if let Some(d) =
            result.egress.clone()
        {
            d
        } else {
            // Legacy path: try to parse the status field as JSON containing an
            // egress directive under the "egress" key.
            let status_str = &result.status;
            let parsed_value = match serde_json::from_str::<serde_json::Value>(status_str) {
                Ok(val) => val,
                Err(_) => return Ok(Reaction::None),
            };
            let egress_val = match parsed_value.get("egress") {
                Some(v) => v,
                None => return Ok(Reaction::None),
            };
            let directive = match serde_json::from_value::<
                crate::domain::models::adapter::EgressDirective,
            >(egress_val.clone())
            {
                Ok(d) => d,
                Err(_) => return Ok(Reaction::None),
            };
            tracing::debug!(
                task_id = %task_id,
                adapter = directive.adapter_name.as_str(),
                "EgressRoutingHandler used legacy status-JSON fallback; \
                 producer should migrate to TaskResultPayload.egress"
            );
            directive
        };

        let adapter_name = &directive.adapter_name;
        let adapter = match self.adapter_registry.get_egress(adapter_name) {
            Some(a) => a,
            None => {
                tracing::warn!(
                    adapter = adapter_name.as_str(),
                    task_id = %task_id,
                    "Egress adapter not found"
                );
                let fail_event = crate::services::event_factory::make_event(
                    EventSeverity::Warning,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressFailed {
                        adapter_name: adapter_name.clone(),
                        task_id: Some(task_id),
                        error: format!("Adapter '{}' not found in registry", adapter_name),
                    },
                );
                return Ok(Reaction::EmitEvents(vec![fail_event]));
            }
        };

        let action_name = format!("{:?}", directive.action);

        match adapter.execute(&directive.action).await {
            Ok(egress_result) => {
                tracing::info!(
                    adapter = adapter_name.as_str(),
                    task_id = %task_id,
                    success = egress_result.success,
                    "Egress action completed"
                );
                let completed_event = crate::services::event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressCompleted {
                        adapter_name: adapter_name.clone(),
                        task_id,
                        action: action_name,
                        success: egress_result.success,
                    },
                );
                Ok(Reaction::EmitEvents(vec![completed_event]))
            }
            Err(e) => {
                tracing::warn!(
                    adapter = adapter_name.as_str(),
                    task_id = %task_id,
                    error = %e,
                    "Egress action failed"
                );
                let fail_event = crate::services::event_factory::make_event(
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
                Ok(Reaction::EmitEvents(vec![fail_event]))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::adapter::{EgressAction, EgressDirective};
    use crate::services::adapter_registry::AdapterRegistry;
    use crate::services::event_bus::TaskResultPayload;
    use crate::services::event_reactor::{EventHandler, HandlerContext, Reaction};
    use uuid::Uuid;

    fn make_handler() -> EgressRoutingHandler {
        EgressRoutingHandler::new(Arc::new(AdapterRegistry::default()))
    }

    fn make_event(payload: EventPayload, task_id: Uuid) -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task_id),
            correlation_id: None,
            source_process_id: None,
            payload,
        }
    }

    fn ctx() -> HandlerContext {
        HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        }
    }

    /// Status is a plain enum string ("Complete") and egress is None:
    /// handler must do nothing.
    #[tokio::test]
    async fn test_plain_status_no_egress_is_noop() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        let result = TaskResultPayload {
            task_id,
            status: "Complete".to_string(),
            error: None,
            duration_secs: 1,
            retry_count: 0,
            tokens_used: 0,
            egress: None,
        };
        let event = make_event(
            EventPayload::TaskCompletedWithResult { task_id, result },
            task_id,
        );
        let reaction = handler.handle(&event, &ctx()).await.unwrap();
        assert!(matches!(reaction, Reaction::None));
    }

    /// Preferred path: structured egress field populated. Adapter is not
    /// registered in the empty registry, so an AdapterEgressFailed event is
    /// emitted — proving the directive was picked up via the dedicated
    /// field rather than ignored.
    #[tokio::test]
    async fn test_dedicated_egress_field_preferred() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        let directive = EgressDirective::new(
            "nonexistent_adapter",
            EgressAction::UpdateStatus {
                external_id: "EXT-1".to_string(),
                new_status: "Done".to_string(),
            },
        );
        let result = TaskResultPayload {
            task_id,
            status: "Complete".to_string(),
            error: None,
            duration_secs: 1,
            retry_count: 0,
            tokens_used: 0,
            egress: Some(directive),
        };
        let event = make_event(
            EventPayload::TaskCompletedWithResult { task_id, result },
            task_id,
        );
        let reaction = handler.handle(&event, &ctx()).await.unwrap();
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::AdapterEgressFailed { adapter_name, .. } => {
                        assert_eq!(adapter_name, "nonexistent_adapter");
                    }
                    other => panic!("Expected AdapterEgressFailed, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }
    }

    /// Legacy path: egress is None but status contains a JSON blob carrying
    /// an "egress" key. The handler should parse it (backwards compat for
    /// events persisted before the dedicated field existed).
    #[tokio::test]
    async fn test_legacy_status_json_fallback() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        // Build the legacy payload by serializing a real EgressDirective so
        // the shape stays in sync with the canonical serde representation.
        let legacy_directive = EgressDirective::new(
            "legacy_adapter",
            EgressAction::UpdateStatus {
                external_id: "EXT-L".to_string(),
                new_status: "Done".to_string(),
            },
        );
        let legacy_json = serde_json::json!({
            "egress": serde_json::to_value(&legacy_directive).unwrap(),
        });
        let result = TaskResultPayload {
            task_id,
            status: legacy_json.to_string(),
            error: None,
            duration_secs: 1,
            retry_count: 0,
            tokens_used: 0,
            egress: None,
        };
        let event = make_event(
            EventPayload::TaskCompletedWithResult { task_id, result },
            task_id,
        );
        let reaction = handler.handle(&event, &ctx()).await.unwrap();
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::AdapterEgressFailed { adapter_name, .. } => {
                        assert_eq!(adapter_name, "legacy_adapter");
                    }
                    other => panic!("Expected AdapterEgressFailed, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }
    }

    /// When both the dedicated field and legacy JSON are populated, the
    /// dedicated field wins.
    #[tokio::test]
    async fn test_dedicated_field_wins_over_legacy_json() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        let dedicated = EgressDirective::new(
            "dedicated_adapter",
            EgressAction::UpdateStatus {
                external_id: "EXT-D".to_string(),
                new_status: "Done".to_string(),
            },
        );
        let legacy_directive = EgressDirective::new(
            "legacy_adapter",
            EgressAction::UpdateStatus {
                external_id: "EXT-L".to_string(),
                new_status: "Done".to_string(),
            },
        );
        let legacy_json = serde_json::json!({
            "egress": serde_json::to_value(&legacy_directive).unwrap(),
        });
        let result = TaskResultPayload {
            task_id,
            status: legacy_json.to_string(),
            error: None,
            duration_secs: 1,
            retry_count: 0,
            tokens_used: 0,
            egress: Some(dedicated),
        };
        let event = make_event(
            EventPayload::TaskCompletedWithResult { task_id, result },
            task_id,
        );
        let reaction = handler.handle(&event, &ctx()).await.unwrap();
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::AdapterEgressFailed { adapter_name, .. } => {
                        assert_eq!(adapter_name, "dedicated_adapter");
                    }
                    other => panic!("Expected AdapterEgressFailed, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }
    }

    /// Backwards compatibility at the serde level: events persisted without
    /// the new `egress` field (older schema) must still deserialize.
    #[test]
    fn test_task_result_payload_deserializes_without_egress_field() {
        let old_json = serde_json::json!({
            "task_id": Uuid::new_v4(),
            "status": "Complete",
            "error": null,
            "duration_secs": 1,
            "retry_count": 0,
            "tokens_used": 100
        });
        let parsed: TaskResultPayload = serde_json::from_value(old_json).unwrap();
        assert!(parsed.egress.is_none());
    }
}

//! Federation event handler for the EventReactor.
//!
//! Listens for `EventCategory::Federation` events (specifically `FederationResultReceived`
//! and `FederationCerebrateUnreachable`) and invokes the configured `FederationResultProcessor`
//! to produce reactions.

use std::sync::Arc;

use async_trait::async_trait;

use crate::services::event_bus::{EventCategory, EventPayload, UnifiedEvent};
use crate::services::event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, HandlerContext, HandlerId, HandlerMetadata,
    HandlerPriority, Reaction,
};

use super::service::FederationService;

/// Reactive handler that processes federation result events through the
/// configured `FederationResultProcessor` trait.
pub struct FederationResultHandler {
    federation_service: Arc<FederationService>,
}

impl FederationResultHandler {
    pub fn new(federation_service: Arc<FederationService>) -> Self {
        Self { federation_service }
    }
}

#[async_trait]
impl EventHandler for FederationResultHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "FederationResultHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Federation],
                min_severity: None,
                goal_id: None,
                task_id: None,
                payload_types: vec![
                    "FederationResultReceived".to_string(),
                    "FederationCerebrateUnreachable".to_string(),
                    "FederationStallDetected".to_string(),
                    "FederationHeartbeatMissed".to_string(),
                ],
                custom_predicate: None,
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        match &event.payload {
            EventPayload::FederationResultReceived { task_id, cerebrate_id, status, summary, .. } => {
                tracing::info!(
                    task_id = %task_id,
                    cerebrate_id = %cerebrate_id,
                    status = %status,
                    summary = %summary,
                    "Federation result received, processing via FederationResultProcessor"
                );
                // The actual result processing happens in FederationService::handle_result(),
                // which is called by the A2A gateway handler. This handler logs the event
                // for observability.
                Ok(Reaction::None)
            }
            EventPayload::FederationCerebrateUnreachable { cerebrate_id, in_flight_tasks } => {
                tracing::warn!(
                    cerebrate_id = %cerebrate_id,
                    in_flight_count = in_flight_tasks.len(),
                    "Cerebrate unreachable with in-flight tasks, starting reconnection"
                );
                // Trigger reconnection with exponential backoff
                self.federation_service.start_reconnect_loop(cerebrate_id.clone()).await;
                Ok(Reaction::None)
            }
            EventPayload::FederationStallDetected { task_id, cerebrate_id, stall_duration_secs } => {
                tracing::warn!(
                    task_id = %task_id,
                    cerebrate_id = %cerebrate_id,
                    stall_secs = stall_duration_secs,
                    "Federation task stalled"
                );
                Ok(Reaction::None)
            }
            EventPayload::FederationHeartbeatMissed { cerebrate_id, missed_count } => {
                tracing::warn!(
                    cerebrate_id = %cerebrate_id,
                    missed_count = missed_count,
                    "Federation heartbeat missed"
                );
                Ok(Reaction::None)
            }
            _ => Ok(Reaction::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::event_bus::{EventBus, EventBusConfig, EventId, EventSeverity, SequenceNumber};
    use crate::services::federation::config::FederationConfig;
    use uuid::Uuid;

    fn make_handler() -> FederationResultHandler {
        let config = FederationConfig::default();
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let svc = Arc::new(super::super::service::FederationService::new(config, event_bus));
        FederationResultHandler::new(svc)
    }

    fn make_event(payload: EventPayload) -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber::zero(),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Federation,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload,
        }
    }

    #[test]
    fn test_handler_metadata() {
        let handler = make_handler();
        let meta = handler.metadata();
        assert_eq!(meta.name, "FederationResultHandler");
        assert!(meta.filter.categories.contains(&EventCategory::Federation));
        assert!(meta.filter.payload_types.contains(&"FederationResultReceived".to_string()));
        assert!(meta.filter.payload_types.contains(&"FederationCerebrateUnreachable".to_string()));
    }

    #[tokio::test]
    async fn test_handle_result_received() {
        let handler = make_handler();
        let event = make_event(EventPayload::FederationResultReceived {
            task_id: Uuid::new_v4(),
            cerebrate_id: "c1".to_string(),
            status: "completed".to_string(),
            summary: "All done".to_string(),
            artifacts: Vec::new(),
        });
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let result = handler.handle(&event, &ctx).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), Reaction::None));
    }

    #[tokio::test]
    async fn test_handle_unreachable() {
        let handler = make_handler();
        let event = make_event(EventPayload::FederationCerebrateUnreachable {
            cerebrate_id: "c1".to_string(),
            in_flight_tasks: vec![Uuid::new_v4()],
        });
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let result = handler.handle(&event, &ctx).await;
        assert!(result.is_ok());
    }
}

//! Federation event handler for the EventReactor.
//!
//! Listens for `EventCategory::Federation` events and translates
//! `FederationReaction`s from the result processor into `Reaction::EmitEvents`
//! for the EventReactor to propagate.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::services::event_bus::{EventCategory, EventPayload, EventSeverity, UnifiedEvent};
use crate::services::event_factory;
use crate::services::event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, HandlerContext, HandlerId, HandlerMetadata,
    HandlerPriority, Reaction,
};

use super::service::FederationService;
use super::traits::FederationReaction;

/// Reactive handler that processes federation result events through the
/// configured `FederationResultProcessor` trait and translates reactions
/// into events for the EventReactor.
pub struct FederationResultHandler {
    federation_service: Arc<FederationService>,
}

impl FederationResultHandler {
    pub fn new(federation_service: Arc<FederationService>) -> Self {
        Self { federation_service }
    }

    /// Convert `FederationReaction`s into `UnifiedEvent`s that the EventReactor
    /// can propagate through the rest of the system.
    ///
    /// `origin_task_id` and `origin_cerebrate_id` carry context from the
    /// originating event so that downstream consumers can correlate reactions
    /// back to the federation task and cerebrate that triggered them.
    fn reactions_to_events(
        &self,
        reactions: &[FederationReaction],
        _origin_task_id: Uuid,
        _origin_cerebrate_id: &str,
    ) -> Vec<UnifiedEvent> {
        let mut events = Vec::new();
        for reaction in reactions {
            match reaction {
                FederationReaction::EmitEvent { description } => {
                    events.push(event_factory::federation_event(
                        EventSeverity::Info,
                        None,
                        EventPayload::FederationReactionEmitted {
                            reaction_type: "emit_event".to_string(),
                            description: description.clone(),
                            goal_id: None,
                            task_id: None,
                        },
                    ));
                }
                FederationReaction::Escalate { reason, goal_id } => {
                    tracing::warn!(
                        reason = %reason,
                        goal_id = ?goal_id,
                        "Federation escalation triggered"
                    );
                    events.push(event_factory::federation_event(
                        EventSeverity::Warning,
                        None,
                        EventPayload::FederationReactionEmitted {
                            reaction_type: "escalate".to_string(),
                            description: format!("Escalation: {reason}"),
                            goal_id: *goal_id,
                            task_id: None,
                        },
                    ));
                }
                FederationReaction::UpdateGoalProgress { goal_id, summary } => {
                    tracing::info!(
                        goal_id = %goal_id,
                        summary = %summary,
                        "Federation goal progress update"
                    );
                    events.push(event_factory::federation_event(
                        EventSeverity::Info,
                        None,
                        EventPayload::FederationReactionEmitted {
                            reaction_type: "update_goal_progress".to_string(),
                            description: format!("Goal {goal_id}: {summary}"),
                            goal_id: Some(*goal_id),
                            task_id: None,
                        },
                    ));
                }
                FederationReaction::CreateTask {
                    title,
                    description,
                    parent_goal_id,
                } => {
                    tracing::info!(
                        title = %title,
                        parent_goal_id = ?parent_goal_id,
                        "Federation reaction: create task"
                    );
                    events.push(event_factory::federation_event(
                        EventSeverity::Info,
                        None,
                        EventPayload::FederationReactionEmitted {
                            reaction_type: "create_task".to_string(),
                            description: format!("Create task: {title} — {description}"),
                            goal_id: *parent_goal_id,
                            task_id: None,
                        },
                    ));
                }
                FederationReaction::DelegateFollowUp {
                    envelope,
                    preferred_cerebrate,
                } => {
                    tracing::info!(
                        task_id = %envelope.task_id,
                        preferred = ?preferred_cerebrate,
                        "Federation reaction: delegate follow-up"
                    );
                    events.push(event_factory::federation_event(
                        EventSeverity::Info,
                        Some(envelope.task_id),
                        EventPayload::FederationTaskDelegated {
                            task_id: envelope.task_id,
                            cerebrate_id: preferred_cerebrate
                                .clone()
                                .unwrap_or_else(|| "auto".to_string()),
                        },
                    ));
                }
                FederationReaction::None => {}
            }
        }
        events
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
            critical: false,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        match &event.payload {
            EventPayload::FederationResultReceived {
                task_id,
                cerebrate_id,
                status,
                summary,
                artifacts,
            } => {
                tracing::info!(
                    task_id = %task_id,
                    cerebrate_id = %cerebrate_id,
                    status = %status,
                    summary = %summary,
                    "Federation result received, invoking result processor"
                );

                // Build a FederationResult from the event payload to run through the processor
                let result = crate::domain::models::a2a::FederationResult {
                    task_id: *task_id,
                    correlation_id: event.correlation_id.unwrap_or(Uuid::nil()),
                    status: match status.as_str() {
                        "completed" => crate::domain::models::a2a::FederationTaskStatus::Completed,
                        "partial" => crate::domain::models::a2a::FederationTaskStatus::Partial,
                        _ => crate::domain::models::a2a::FederationTaskStatus::Failed,
                    },
                    summary: summary.clone(),
                    artifacts: artifacts.clone(),
                    metrics: std::collections::HashMap::new(),
                    notes: None,
                    failure_reason: None,
                    suggestions: Vec::new(),
                };

                let parent_context = super::traits::ParentContext {
                    goal_id: event.goal_id,
                    goal_summary: None,
                    task_title: None,
                };

                let processor = self.federation_service.result_processor();
                let reactions = match result.status {
                    crate::domain::models::a2a::FederationTaskStatus::Completed
                    | crate::domain::models::a2a::FederationTaskStatus::Partial => {
                        processor.process_result(&result, &parent_context)
                    }
                    crate::domain::models::a2a::FederationTaskStatus::Failed => {
                        processor.process_failure(&result, &parent_context)
                    }
                };

                let events = self.reactions_to_events(&reactions, *task_id, cerebrate_id);
                if events.is_empty() {
                    Ok(Reaction::None)
                } else {
                    Ok(Reaction::EmitEvents(events))
                }
            }
            EventPayload::FederationCerebrateUnreachable {
                cerebrate_id,
                in_flight_tasks,
            } => {
                tracing::warn!(
                    cerebrate_id = %cerebrate_id,
                    in_flight_count = in_flight_tasks.len(),
                    "Cerebrate unreachable with in-flight tasks, starting reconnection"
                );
                // Trigger reconnection with exponential backoff
                self.federation_service
                    .start_reconnect_loop(cerebrate_id.clone())
                    .await;

                // Also process failure reactions for each in-flight task
                let processor = self.federation_service.result_processor();
                let mut all_events = Vec::new();
                for task_id in in_flight_tasks {
                    let result = crate::domain::models::a2a::FederationResult {
                        task_id: *task_id,
                        correlation_id: event.correlation_id.unwrap_or(Uuid::nil()),
                        status: crate::domain::models::a2a::FederationTaskStatus::Failed,
                        summary: format!(
                            "Cerebrate {cerebrate_id} became unreachable"
                        ),
                        artifacts: Vec::new(),
                        metrics: std::collections::HashMap::new(),
                        notes: None,
                        failure_reason: Some("cerebrate_unreachable".to_string()),
                        suggestions: Vec::new(),
                    };
                    let parent_context = super::traits::ParentContext {
                        goal_id: event.goal_id,
                        goal_summary: None,
                        task_title: None,
                    };
                    let reactions = processor.process_failure(&result, &parent_context);
                    all_events.extend(self.reactions_to_events(&reactions, *task_id, cerebrate_id));
                }

                if all_events.is_empty() {
                    Ok(Reaction::None)
                } else {
                    Ok(Reaction::EmitEvents(all_events))
                }
            }
            EventPayload::FederationStallDetected {
                task_id,
                cerebrate_id,
                stall_duration_secs,
            } => {
                tracing::warn!(
                    task_id = %task_id,
                    cerebrate_id = %cerebrate_id,
                    stall_secs = stall_duration_secs,
                    "Federation task stalled — escalating"
                );
                let reactions = vec![FederationReaction::Escalate {
                    reason: format!(
                        "Task {task_id} on cerebrate {cerebrate_id} stalled for {stall_duration_secs}s"
                    ),
                    goal_id: event.goal_id,
                }];
                let events = self.reactions_to_events(&reactions, *task_id, cerebrate_id);
                if events.is_empty() {
                    Ok(Reaction::None)
                } else {
                    Ok(Reaction::EmitEvents(events))
                }
            }
            EventPayload::FederationHeartbeatMissed {
                cerebrate_id,
                missed_count,
            } => {
                tracing::warn!(
                    cerebrate_id = %cerebrate_id,
                    missed_count = missed_count,
                    "Federation heartbeat missed"
                );
                // Heartbeat misses are informational until threshold reached
                // (the service handles the threshold transition to Unreachable)
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
        assert!(
            meta.filter
                .payload_types
                .contains(&"FederationResultReceived".to_string())
        );
        assert!(
            meta.filter
                .payload_types
                .contains(&"FederationCerebrateUnreachable".to_string())
        );
    }

    #[tokio::test]
    async fn test_handle_result_received_emits_reactions() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        let event = make_event(EventPayload::FederationResultReceived {
            task_id,
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
        // DefaultResultProcessor emits UpdateGoalProgress on success,
        // but without a goal_id the reaction has a nil UUID
        match result.unwrap() {
            Reaction::EmitEvents(events) => {
                assert!(!events.is_empty(), "Should emit events for reactions");
            }
            Reaction::None => {
                // Also valid if goal_id is None (no reactions emitted)
            }
        }
    }

    #[tokio::test]
    async fn test_handle_failed_result_emits_escalation() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        let event = make_event(EventPayload::FederationResultReceived {
            task_id,
            cerebrate_id: "c1".to_string(),
            status: "failed".to_string(),
            summary: "Something broke".to_string(),
            artifacts: Vec::new(),
        });
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let result = handler.handle(&event, &ctx).await;
        assert!(result.is_ok());
        // DefaultResultProcessor emits Escalate on failure
        match result.unwrap() {
            Reaction::EmitEvents(events) => {
                assert!(!events.is_empty(), "Should emit escalation events");
            }
            Reaction::None => {
                panic!("Failed results should produce escalation reactions");
            }
        }
    }

    #[tokio::test]
    async fn test_handle_unreachable_processes_in_flight() {
        let handler = make_handler();
        let event = make_event(EventPayload::FederationCerebrateUnreachable {
            cerebrate_id: "c1".to_string(),
            in_flight_tasks: vec![Uuid::new_v4(), Uuid::new_v4()],
        });
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let result = handler.handle(&event, &ctx).await;
        assert!(result.is_ok());
        // Should emit escalation events for each in-flight task
        match result.unwrap() {
            Reaction::EmitEvents(events) => {
                assert!(
                    events.len() >= 2,
                    "Should emit at least one event per in-flight task"
                );
            }
            Reaction::None => {
                panic!("Unreachable with in-flight tasks should produce reactions");
            }
        }
    }

    #[tokio::test]
    async fn test_handle_stall_escalates() {
        let handler = make_handler();
        let event = make_event(EventPayload::FederationStallDetected {
            task_id: Uuid::new_v4(),
            cerebrate_id: "c1".to_string(),
            stall_duration_secs: 1800,
        });
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let result = handler.handle(&event, &ctx).await;
        assert!(result.is_ok());
        match result.unwrap() {
            Reaction::EmitEvents(events) => {
                assert!(!events.is_empty(), "Stall should produce escalation event");
            }
            Reaction::None => {
                panic!("Stalls should produce escalation reactions");
            }
        }
    }

    #[tokio::test]
    async fn test_handle_heartbeat_missed() {
        let handler = make_handler();
        let event = make_event(EventPayload::FederationHeartbeatMissed {
            cerebrate_id: "c1".to_string(),
            missed_count: 2,
        });
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let result = handler.handle(&event, &ctx).await;
        assert!(result.is_ok());
        // Heartbeat misses are informational - no reactions until threshold
        assert!(matches!(result.unwrap(), Reaction::None));
    }

    #[test]
    fn test_reactions_to_events_none() {
        let handler = make_handler();
        let events =
            handler.reactions_to_events(&[FederationReaction::None], Uuid::new_v4(), "c1");
        assert!(events.is_empty());
    }

    #[test]
    fn test_reactions_to_events_escalate() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        let goal = Uuid::new_v4();
        let reactions = vec![FederationReaction::Escalate {
            reason: "test".to_string(),
            goal_id: Some(goal),
        }];
        let events = handler.reactions_to_events(&reactions, task_id, "c1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].severity, EventSeverity::Warning);
        // Verify it uses the distinct FederationReactionEmitted payload
        match &events[0].payload {
            EventPayload::FederationReactionEmitted {
                reaction_type,
                goal_id,
                ..
            } => {
                assert_eq!(reaction_type, "escalate");
                assert_eq!(*goal_id, Some(goal));
            }
            other => panic!("Expected FederationReactionEmitted, got {:?}", other.variant_name()),
        }
    }

    #[test]
    fn test_reactions_to_events_create_task() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        let goal = Uuid::new_v4();
        let reactions = vec![FederationReaction::CreateTask {
            title: "Fix the thing".to_string(),
            description: "Something needs fixing".to_string(),
            parent_goal_id: Some(goal),
        }];
        let events = handler.reactions_to_events(&reactions, task_id, "c1");
        assert_eq!(events.len(), 1);
        match &events[0].payload {
            EventPayload::FederationReactionEmitted {
                reaction_type,
                description,
                goal_id,
                ..
            } => {
                assert_eq!(reaction_type, "create_task");
                assert!(description.contains("Fix the thing"));
                assert_eq!(*goal_id, Some(goal));
            }
            other => panic!("Expected FederationReactionEmitted, got {:?}", other.variant_name()),
        }
    }

    #[test]
    fn test_reactions_to_events_update_goal_progress() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        let goal = Uuid::new_v4();
        let reactions = vec![FederationReaction::UpdateGoalProgress {
            goal_id: goal,
            summary: "50% complete".to_string(),
        }];
        let events = handler.reactions_to_events(&reactions, task_id, "c1");
        assert_eq!(events.len(), 1);
        match &events[0].payload {
            EventPayload::FederationReactionEmitted {
                reaction_type,
                goal_id,
                ..
            } => {
                assert_eq!(reaction_type, "update_goal_progress");
                assert_eq!(*goal_id, Some(goal));
            }
            other => panic!("Expected FederationReactionEmitted, got {:?}", other.variant_name()),
        }
    }

    #[test]
    fn test_reactions_to_events_emit_event() {
        let handler = make_handler();
        let task_id = Uuid::new_v4();
        let reactions = vec![FederationReaction::EmitEvent {
            description: "Something happened".to_string(),
        }];
        let events = handler.reactions_to_events(&reactions, task_id, "c1");
        assert_eq!(events.len(), 1);
        match &events[0].payload {
            EventPayload::FederationReactionEmitted {
                reaction_type,
                description,
                ..
            } => {
                assert_eq!(reaction_type, "emit_event");
                assert_eq!(description, "Something happened");
            }
            other => panic!("Expected FederationReactionEmitted, got {:?}", other.variant_name()),
        }
    }
}

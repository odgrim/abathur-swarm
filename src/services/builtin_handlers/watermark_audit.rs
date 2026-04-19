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
// WatermarkAuditHandler
// ============================================================================

/// Triggered by the "watermark-audit" scheduled event (600s).
/// Reads all handler watermarks from the event store, compares them to the
/// latest event sequence, and logs warnings for handlers that are
/// significantly behind (>100 events).
pub struct WatermarkAuditHandler {
    event_store: Arc<dyn EventStore>,
    /// Names of handlers to audit (snapshot taken at registration time).
    handler_names: Vec<String>,
}

impl WatermarkAuditHandler {
    pub fn new(event_store: Arc<dyn EventStore>, handler_names: Vec<String>) -> Self {
        Self {
            event_store,
            handler_names,
        }
    }
}

#[async_trait]
impl EventHandler for WatermarkAuditHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "WatermarkAuditHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "watermark-audit"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        _event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let latest = self
            .event_store
            .latest_sequence()
            .await
            .map_err(|e| format!("WatermarkAudit: failed to get latest sequence: {}", e))?;

        let latest_seq = match latest {
            Some(seq) => seq.0,
            None => return Ok(Reaction::None), // No events in store yet
        };

        let mut behind_count = 0u32;
        let mut max_lag: u64 = 0;
        let mut new_events = Vec::new();

        for name in &self.handler_names {
            let wm = self.event_store.get_watermark(name).await.map_err(|e| {
                format!(
                    "WatermarkAudit: failed to get watermark for {}: {}",
                    name, e
                )
            })?;

            let handler_seq = wm.map(|s| s.0).unwrap_or(0);
            let lag = latest_seq.saturating_sub(handler_seq);

            if lag > max_lag {
                max_lag = lag;
            }

            if lag > 100 {
                tracing::warn!(
                    handler = %name,
                    handler_seq = handler_seq,
                    latest_seq = latest_seq,
                    lag = lag,
                    "WatermarkAudit: handler is significantly behind"
                );
                behind_count += 1;
            }
        }

        if behind_count > 0 {
            tracing::info!(
                "WatermarkAudit: {} handler(s) significantly behind latest sequence {}",
                behind_count,
                latest_seq
            );

            // When lag > 100: trigger a catch-up sweep
            if max_lag > 100 {
                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Info,
                    category: EventCategory::Scheduler,
                    goal_id: None,
                    task_id: None,
                    correlation_id: None,
                    source_process_id: None,
                    payload: EventPayload::ScheduledEventFired {
                        schedule_id: uuid::Uuid::new_v4(),
                        name: "trigger-rule-catchup".to_string(),
                    },
                });
            }

            // When lag > 500: emit a human escalation event
            if max_lag > 500 {
                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Escalation,
                    goal_id: None,
                    task_id: None,
                    correlation_id: None,
                    source_process_id: None,
                    payload: EventPayload::HumanEscalationRequired(HumanEscalationPayload {
                        goal_id: None,
                        task_id: None,
                        reason: format!(
                            "Event processing critically behind: {} handler(s) lagging, max lag {} events",
                            behind_count, max_lag
                        ),
                        urgency: "high".to_string(),
                        questions: vec![
                            "Event handlers are critically behind. Should the system be restarted or investigated?".to_string(),
                        ],
                        is_blocking: false,
                    }),
                });
            }
        } else {
            tracing::debug!(
                "WatermarkAudit: all handlers within 100 events of sequence {}",
                latest_seq
            );
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

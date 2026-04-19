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
// EscalationTimeoutHandler
// ============================================================================

/// Triggered by the "escalation-check" scheduled event. Emits a notification
/// that escalation deadlines should be checked. The actual timeout logic is
/// handled by the poll-based `check_escalation_deadlines` in the orchestrator,
/// since escalation state lives in the orchestrator's in-memory store.
///
/// This handler provides a fast-path signal: when it fires, the orchestrator
/// can immediately check deadlines rather than waiting for the next poll tick.
pub struct EscalationTimeoutHandler {
    escalation_store: Arc<RwLock<Vec<HumanEscalationEvent>>>,
}

impl EscalationTimeoutHandler {
    pub fn new(escalation_store: Arc<RwLock<Vec<HumanEscalationEvent>>>) -> Self {
        Self { escalation_store }
    }
}

#[async_trait]
impl EventHandler for EscalationTimeoutHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EscalationTimeoutHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "escalation-check"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        // Check escalation deadlines from the shared store
        let now = chrono::Utc::now();
        let store = self.escalation_store.read().await;
        let expired: Vec<_> = store
            .iter()
            .filter(|e| e.escalation.deadline.is_some_and(|d| now > d))
            .cloned()
            .collect();
        drop(store);

        if expired.is_empty() {
            return Ok(Reaction::None);
        }

        tracing::info!(
            "EscalationTimeoutHandler: {} escalation(s) past deadline",
            expired.len()
        );

        let mut new_events = Vec::new();
        for esc in &expired {
            let default_action = esc
                .escalation
                .default_action
                .as_deref()
                .unwrap_or("timeout-logged")
                .to_string();

            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Warning,
                category: EventCategory::Escalation,
                goal_id: esc.goal_id,
                task_id: esc.task_id,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::HumanEscalationExpired {
                    task_id: esc.task_id,
                    goal_id: esc.goal_id,
                    default_action,
                },
            });
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

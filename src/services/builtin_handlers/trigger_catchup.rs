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
// TriggerCatchupHandler
// ============================================================================

/// Triggered by the "trigger-rule-catchup" scheduled event (300s).
/// Re-evaluates events that the TriggerRuleEngine may have missed by reading
/// its own watermark and replaying events since that point.
pub struct TriggerCatchupHandler {
    trigger_engine: Arc<crate::services::trigger_rules::TriggerRuleEngine>,
    event_store: Arc<dyn EventStore>,
}

impl TriggerCatchupHandler {
    pub fn new(
        trigger_engine: Arc<crate::services::trigger_rules::TriggerRuleEngine>,
        event_store: Arc<dyn EventStore>,
    ) -> Self {
        Self {
            trigger_engine,
            event_store,
        }
    }
}

#[async_trait]
impl EventHandler for TriggerCatchupHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TriggerCatchupHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "trigger-rule-catchup"
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
        ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        // Read the TriggerRuleEngine's own watermark
        let wm = self
            .event_store
            .get_watermark("TriggerRuleEngine")
            .await
            .map_err(|e| format!("TriggerCatchup: failed to get watermark: {}", e))?;

        let since_seq = wm.unwrap_or(crate::services::event_bus::SequenceNumber(0));

        // Query events since that watermark
        let events = self
            .event_store
            .replay_since(since_seq)
            .await
            .map_err(|e| format!("TriggerCatchup: failed to replay events: {}", e))?;

        if events.is_empty() {
            return Ok(Reaction::None);
        }

        let mut all_reactions = Vec::new();
        let mut max_seq = since_seq;

        let handler_ctx = HandlerContext {
            chain_depth: ctx.chain_depth,
            correlation_id: ctx.correlation_id,
        };

        for evt in &events {
            // Skip the catchup event itself to avoid infinite loops
            if matches!(&evt.payload, EventPayload::ScheduledEventFired { name, .. } if name == "trigger-rule-catchup")
            {
                continue;
            }

            // Re-evaluate through the trigger engine
            match self.trigger_engine.handle(evt, &handler_ctx).await {
                Ok(Reaction::EmitEvents(new_events)) => {
                    all_reactions.extend(new_events);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        "TriggerCatchup: trigger engine error on seq {}: {}",
                        evt.sequence,
                        e
                    );
                }
            }

            if evt.sequence > max_seq {
                max_seq = evt.sequence;
            }
        }

        // Update watermark after processing
        if max_seq > since_seq
            && let Err(e) = self
                .event_store
                .set_watermark("TriggerRuleEngine", max_seq)
                .await
        {
            tracing::warn!("TriggerCatchup: failed to update watermark: {}", e);
        }

        tracing::debug!(
            events_replayed = events.len(),
            reactions = all_reactions.len(),
            "TriggerCatchup: catch-up sweep complete"
        );

        if all_reactions.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(all_reactions))
        }
    }
}

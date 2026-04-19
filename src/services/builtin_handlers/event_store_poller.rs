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
// EventStorePollerHandler
// ============================================================================

/// Triggered by the "event-store-poll" scheduled event.
/// Reads events from the SQLite EventStore with sequence numbers beyond
/// the poller's high-water mark and re-publishes them into the broadcast
/// channel, enabling cross-process event propagation.
///
/// Filters out events originating from the current process (using
/// `source_process_id`) to avoid echo loops.
pub struct EventStorePollerHandler {
    event_store: Arc<dyn EventStore>,
    /// Process ID of the local EventBus — events with this source are skipped.
    local_process_id: uuid::Uuid,
    /// High-water mark: the latest sequence number this poller has seen.
    high_water_mark: Arc<RwLock<u64>>,
    /// Maximum events to process per poll cycle.
    max_per_poll: usize,
}

impl EventStorePollerHandler {
    pub fn new(event_store: Arc<dyn EventStore>, local_process_id: uuid::Uuid) -> Self {
        Self {
            event_store,
            local_process_id,
            high_water_mark: Arc::new(RwLock::new(0)),
            max_per_poll: 100,
        }
    }

    /// Initialize the high-water mark from the event store's latest sequence.
    /// Call this at startup so we don't replay the entire history.
    ///
    /// When no watermark exists (first run), we start from
    /// `max_sequence - replay_window` instead of `max_sequence` to ensure
    /// recent events are replayed for catch-up.
    pub async fn initialize_watermark(&self) {
        self.initialize_watermark_with_replay(1000).await;
    }

    /// Initialize watermark with a configurable replay window.
    pub async fn initialize_watermark_with_replay(&self, replay_window: u64) {
        match self
            .event_store
            .get_watermark("EventStorePollerHandler")
            .await
        {
            Ok(Some(seq)) => {
                let mut hwm = self.high_water_mark.write().await;
                *hwm = seq.0;
                tracing::info!("EventStorePoller: initialized watermark at {}", seq.0);
            }
            Ok(None) => {
                // No watermark yet — start from max_sequence - replay_window to
                // ensure recent events are replayed for catch-up
                if let Ok(Some(seq)) = self.event_store.latest_sequence().await {
                    let start_from = seq.0.saturating_sub(replay_window);
                    let mut hwm = self.high_water_mark.write().await;
                    *hwm = start_from;
                    tracing::info!(
                        "EventStorePoller: no watermark found, starting from seq {} (latest {} - window {})",
                        start_from,
                        seq.0,
                        replay_window
                    );
                }
            }
            Err(e) => {
                tracing::warn!("EventStorePoller: failed to read watermark: {}", e);
            }
        }
    }
}

#[async_trait]
impl EventHandler for EventStorePollerHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EventStorePollerHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "event-store-poll"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::SYSTEM,
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: false,
        }
    }

    async fn handle(
        &self,
        _event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let hwm = {
            let h = self.high_water_mark.read().await;
            *h
        };

        // Query events beyond our high-water mark
        let events = self
            .event_store
            .query(
                crate::services::event_store::EventQuery::new()
                    .since_sequence(SequenceNumber(hwm + 1))
                    .ascending()
                    .limit(self.max_per_poll as u32),
            )
            .await
            .map_err(|e| format!("EventStorePoller: failed to query events: {}", e))?;

        if events.is_empty() {
            return Ok(Reaction::None);
        }

        let mut new_events = Vec::new();
        let mut new_hwm = hwm;

        for evt in &events {
            // Track highest sequence seen
            if evt.sequence.0 > new_hwm {
                new_hwm = evt.sequence.0;
            }

            // Skip events from this process (we already broadcast them)
            if evt.source_process_id == Some(self.local_process_id) {
                continue;
            }

            // Skip ScheduledEventFired — those are generated locally
            if matches!(&evt.payload, EventPayload::ScheduledEventFired { .. }) {
                continue;
            }

            new_events.push(evt.clone());
        }

        // Update high-water mark
        if new_hwm > hwm {
            let mut h = self.high_water_mark.write().await;
            *h = new_hwm;

            // Persist watermark
            if let Err(e) = self
                .event_store
                .set_watermark("EventStorePollerHandler", SequenceNumber(new_hwm))
                .await
            {
                tracing::warn!("EventStorePoller: failed to persist watermark: {}", e);
            }
        }

        if !new_events.is_empty() {
            tracing::info!(
                "EventStorePoller: re-publishing {} cross-process events (hwm {} -> {})",
                new_events.len(),
                hwm,
                new_hwm
            );
            Ok(Reaction::EmitEvents(new_events))
        } else {
            Ok(Reaction::None)
        }
    }
}

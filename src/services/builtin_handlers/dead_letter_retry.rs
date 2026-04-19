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
// DeadLetterRetryHandler
// ============================================================================

/// Triggered by the "dead-letter-retry" scheduled event.
/// Reads retryable entries from the dead letter queue, re-fetches the original
/// event from the store, and re-publishes it so handlers get another chance.
/// Applies exponential backoff (2^retry_count seconds) between retries.
/// Marks entries as resolved when max retries exceeded.
pub struct DeadLetterRetryHandler {
    event_store: Arc<dyn EventStore>,
}

impl DeadLetterRetryHandler {
    pub fn new(event_store: Arc<dyn EventStore>) -> Self {
        Self { event_store }
    }
}

#[async_trait]
impl EventHandler for DeadLetterRetryHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "DeadLetterRetryHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "dead-letter-retry"
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
        let entries = self
            .event_store
            .get_retryable_dead_letters(10)
            .await
            .map_err(|e| format!("DeadLetterRetry: failed to get retryable entries: {}", e))?;

        if entries.is_empty() {
            return Ok(Reaction::None);
        }

        let mut events_to_replay = Vec::new();

        for entry in &entries {
            // If this is the last attempt, resolve it before re-publishing
            if entry.retry_count + 1 >= entry.max_retries {
                tracing::info!(
                    "DeadLetterRetry: max retries ({}) reached for handler '{}' on event seq {}, resolving",
                    entry.max_retries,
                    entry.handler_name,
                    entry.event_sequence
                );
                if let Err(e) = self.event_store.resolve_dead_letter(&entry.id).await {
                    tracing::warn!(
                        "DeadLetterRetry: failed to resolve entry {}: {}",
                        entry.id,
                        e
                    );
                }
            } else {
                // Increment retry count BEFORE re-publishing to prevent duplicates on crash.
                // If re-publish fails, the DLQ entry still exists for next retry.
                let backoff_secs = 2i64.pow((entry.retry_count + 1).min(10));
                let next_retry = chrono::Utc::now() + chrono::Duration::seconds(backoff_secs);

                if let Err(e) = self
                    .event_store
                    .increment_dead_letter_retry(&entry.id, next_retry)
                    .await
                {
                    tracing::warn!(
                        "DeadLetterRetry: failed to increment retry for {}: {}",
                        entry.id,
                        e
                    );
                    continue; // Skip re-publish if we couldn't mark the retry
                }
            }

            // Re-fetch the original event from the store
            let original = self
                .event_store
                .get_by_sequence(SequenceNumber(entry.event_sequence))
                .await
                .map_err(|e| {
                    format!(
                        "DeadLetterRetry: failed to get event seq {}: {}",
                        entry.event_sequence, e
                    )
                })?;

            match original {
                Some(evt) => {
                    events_to_replay.push(evt);
                }
                None => {
                    // Event no longer in store (pruned), resolve the DLQ entry
                    tracing::warn!(
                        "DeadLetterRetry: event seq {} no longer in store (pruned), resolving DLQ entry {} — handler recovery lost for '{}'",
                        entry.event_sequence,
                        entry.id,
                        entry.handler_name
                    );
                    if let Err(e) = self.event_store.resolve_dead_letter(&entry.id).await {
                        tracing::warn!(
                            "DeadLetterRetry: failed to resolve entry {}: {}",
                            entry.id,
                            e
                        );
                    }
                    // Emit a HandlerError event to make the permanent loss visible
                    events_to_replay.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Warning,
                        category: EventCategory::Orchestrator,
                        goal_id: None,
                        task_id: None,
                        correlation_id: None,
                        source_process_id: None,
                        payload: EventPayload::HandlerError {
                            handler_name: entry.handler_name.clone(),
                            event_sequence: entry.event_sequence,
                            error: format!("Original event seq {} pruned before dead-letter retry — handler recovery lost", entry.event_sequence),
                            circuit_breaker_tripped: false,
                        },
                    });
                }
            }
        }

        if events_to_replay.is_empty() {
            Ok(Reaction::None)
        } else {
            tracing::info!(
                "DeadLetterRetry: re-publishing {} events from dead letter queue",
                events_to_replay.len()
            );
            Ok(Reaction::EmitEvents(events_to_replay))
        }
    }
}

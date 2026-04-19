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
// MemoryConflictEscalationHandler (Phase 3b)
// ============================================================================

/// Triggered by `MemoryConflictDetected`. Escalates conflicts that are
/// flagged for review (low similarity) in semantic-tier memories.
pub struct MemoryConflictEscalationHandler;

impl Default for MemoryConflictEscalationHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryConflictEscalationHandler {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EventHandler for MemoryConflictEscalationHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "MemoryConflictEscalationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Memory])
                .payload_types(vec!["MemoryConflictDetected".to_string()]),
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
        let (memory_a, memory_b, key, similarity) = match &event.payload {
            EventPayload::MemoryConflictDetected {
                memory_a,
                memory_b,
                key,
                similarity,
            } => (*memory_a, *memory_b, key.clone(), *similarity),
            _ => return Ok(Reaction::None),
        };

        // Only escalate for low-similarity conflicts (flagged for review)
        if similarity >= 0.3 {
            return Ok(Reaction::None);
        }

        let escalation = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Warning,
            category: EventCategory::Escalation,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::HumanEscalationRequired(HumanEscalationPayload {
                goal_id: None,
                task_id: None,
                reason: format!(
                    "Memory conflict detected for key '{}': memories {} and {} have low similarity ({:.2})",
                    key, memory_a, memory_b, similarity
                ),
                urgency: "high".to_string(),
                questions: vec![format!("Which version of memory '{}' should be kept?", key)],
                is_blocking: true,
            }),
        };

        Ok(Reaction::EmitEvents(vec![escalation]))
    }
}

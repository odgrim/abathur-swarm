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
// BudgetOpportunityHandler
// ============================================================================

/// Converts `BudgetOpportunityDetected` events into synthetic
/// `ScheduledEventFired { name: "goal-convergence-check:budget-trigger" }`
/// events, allowing the `GoalConvergenceCheckHandler` to fire an out-of-band
/// convergence check when budget headroom is available.
pub struct BudgetOpportunityHandler;

impl BudgetOpportunityHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BudgetOpportunityHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler for BudgetOpportunityHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "BudgetOpportunityHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Budget])
                .payload_types(vec!["BudgetOpportunityDetected".to_string()]),
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let opportunity_score = match &event.payload {
            EventPayload::BudgetOpportunityDetected {
                opportunity_score, ..
            } => *opportunity_score,
            _ => return Ok(Reaction::None),
        };

        tracing::info!(
            opportunity_score,
            "BudgetOpportunityHandler: budget opportunity detected, triggering out-of-band convergence check"
        );

        let trigger_event = crate::services::event_factory::make_event(
            EventSeverity::Info,
            EventCategory::Scheduler,
            None,
            None,
            EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "goal-convergence-check:budget-trigger".to_string(),
            },
        );

        Ok(Reaction::EmitEvents(vec![trigger_event]))
    }
}

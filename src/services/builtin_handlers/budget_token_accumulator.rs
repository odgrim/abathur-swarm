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
// BudgetTokenAccumulatorHandler
// ============================================================================

/// Accumulates token usage from `AgentInstanceCompleted` events into the
/// `BudgetTracker` and recomputes aggregate budget pressure.
///
/// This allows the budget system to maintain a running tally of tokens
/// consumed by the swarm without polling external APIs.
pub struct BudgetTokenAccumulatorHandler {
    budget_tracker: Arc<crate::services::budget_tracker::BudgetTracker>,
}

impl BudgetTokenAccumulatorHandler {
    pub fn new(budget_tracker: Arc<crate::services::budget_tracker::BudgetTracker>) -> Self {
        Self { budget_tracker }
    }
}

#[async_trait]
impl EventHandler for BudgetTokenAccumulatorHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "BudgetTokenAccumulatorHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Agent])
                .payload_types(vec!["AgentInstanceCompleted".to_string()]),
            priority: HandlerPriority::SYSTEM,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let (task_id, tokens_used) = match &event.payload {
            EventPayload::AgentInstanceCompleted {
                task_id,
                tokens_used,
                ..
            } => (*task_id, *tokens_used),
            _ => return Ok(Reaction::None),
        };

        self.budget_tracker
            .record_tokens_used(task_id, tokens_used)
            .await;
        self.budget_tracker.recompute_state().await;

        Ok(Reaction::None)
    }
}

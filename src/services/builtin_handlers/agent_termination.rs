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
// AgentTerminationHandler
// ============================================================================

/// When a task fails, terminate the underlying agent subprocess and free
/// the guardrail agent slot.
///
/// Without this, a timed-out task gets retried back to Ready while the
/// original agent process is still running and holding a concurrency slot,
/// starving the scheduler of capacity.
pub struct AgentTerminationHandler {
    substrate: Arc<dyn crate::domain::ports::Substrate>,
    guardrails: Arc<crate::services::guardrails::Guardrails>,
}

impl AgentTerminationHandler {
    pub fn new(
        substrate: Arc<dyn crate::domain::ports::Substrate>,
        guardrails: Arc<crate::services::guardrails::Guardrails>,
    ) -> Self {
        Self {
            substrate,
            guardrails,
        }
    }
}

#[async_trait]
impl EventHandler for AgentTerminationHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "AgentTerminationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskFailed".to_string()]),
            priority: HandlerPriority::SYSTEM,
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: true,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let task_id = match &event.payload {
            EventPayload::TaskFailed { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Kill the agent subprocess (no-op if already exited)
        if let Err(e) = self.substrate.terminate_by_task_id(task_id).await {
            tracing::warn!(
                task_id = %task_id,
                error = %e,
                "AgentTerminationHandler: failed to terminate agent subprocess"
            );
        }

        // Immediately free the guardrail slot so the scheduler can
        // dispatch new work without waiting for the killed process
        // to fully wind down. Double-deregistration is safe (HashMap::remove
        // on a missing key is a no-op).
        self.guardrails
            .register_agent_end(&task_id.to_string())
            .await;

        Ok(Reaction::None)
    }
}

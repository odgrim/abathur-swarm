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
// GoalRetiredHandler
// ============================================================================

/// When a goal is retired, invalidate the active goals cache and emit a
/// summary event. Does not cancel tasks (goals and tasks are decoupled
/// in this architecture — tasks do not carry a goal_id field).
pub struct GoalRetiredHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    active_goals_cache: Arc<RwLock<Vec<Goal>>>,
}

impl<G: GoalRepository> GoalRetiredHandler<G> {
    pub fn new(goal_repo: Arc<G>, active_goals_cache: Arc<RwLock<Vec<Goal>>>) -> Self {
        Self {
            goal_repo,
            active_goals_cache,
        }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for GoalRetiredHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalRetiredHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Goal],
                payload_types: vec!["GoalStatusChanged".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::GoalStatusChanged { to_status, .. } if to_status == "retired"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let goal_id = match event.goal_id {
            Some(id) => id,
            None => return Ok(Reaction::None),
        };

        tracing::info!(
            "GoalRetiredHandler: goal {} retired, refreshing active goals cache",
            goal_id
        );

        // Refresh the active goals cache to exclude the retired goal
        let goals = self
            .goal_repo
            .get_active_with_constraints()
            .await
            .map_err(|e| format!("Failed to refresh active goals: {}", e))?;
        {
            let mut cache = self.active_goals_cache.write().await;
            *cache = goals;
        }

        // Emit a GoalStatusChanged event is already the triggering event;
        // we log for observability and emit no additional events.
        Ok(Reaction::None)
    }
}

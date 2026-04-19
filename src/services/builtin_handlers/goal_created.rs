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
// GoalCreatedHandler
// ============================================================================

/// When a goal starts, refresh the active goals cache.
pub struct GoalCreatedHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    active_goals_cache: Arc<RwLock<Vec<Goal>>>,
}

impl<G: GoalRepository> GoalCreatedHandler<G> {
    pub fn new(goal_repo: Arc<G>, active_goals_cache: Arc<RwLock<Vec<Goal>>>) -> Self {
        Self {
            goal_repo,
            active_goals_cache,
        }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for GoalCreatedHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalCreatedHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Goal])
                .payload_types(vec!["GoalStarted".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        _event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let goals = self
            .goal_repo
            .get_active_with_constraints()
            .await
            .map_err(|e| format!("Failed to get active goals: {}", e))?;

        let mut cache = self.active_goals_cache.write().await;
        *cache = goals;

        Ok(Reaction::None)
    }
}

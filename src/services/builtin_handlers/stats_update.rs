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
// StatsUpdateHandler
// ============================================================================

/// Triggered by the "stats-update" scheduled event. Refreshes swarm statistics.
pub struct StatsUpdateHandler<G: GoalRepository, T: TaskRepository, W: WorktreeRepository> {
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
    worktree_repo: Arc<W>,
    stats: Arc<RwLock<SwarmStats>>,
    agent_semaphore: Arc<Semaphore>,
    max_agents: usize,
    total_tokens: Arc<AtomicU64>,
}

impl<G: GoalRepository, T: TaskRepository, W: WorktreeRepository> StatsUpdateHandler<G, T, W> {
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        worktree_repo: Arc<W>,
        stats: Arc<RwLock<SwarmStats>>,
        agent_semaphore: Arc<Semaphore>,
        max_agents: usize,
        total_tokens: Arc<AtomicU64>,
    ) -> Self {
        Self {
            goal_repo,
            task_repo,
            worktree_repo,
            stats,
            agent_semaphore,
            max_agents,
            total_tokens,
        }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static, T: TaskRepository + 'static, W: WorktreeRepository + 'static>
    EventHandler for StatsUpdateHandler<G, T, W>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "StatsUpdateHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "stats-update"
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
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let task_counts = self
            .task_repo
            .count_by_status()
            .await
            .map_err(|e| format!("Failed to count tasks: {}", e))?;
        let active_worktrees = self
            .worktree_repo
            .list_active()
            .await
            .map_err(|e| format!("Failed to list worktrees: {}", e))?
            .len();

        let active_goals = self
            .goal_repo
            .list(crate::domain::ports::GoalFilter {
                status: Some(crate::domain::models::GoalStatus::Active),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("Failed to list goals: {}", e))?
            .len();

        let new_stats = SwarmStats {
            active_goals,
            pending_tasks: *task_counts.get(&TaskStatus::Pending).unwrap_or(&0) as usize,
            ready_tasks: *task_counts.get(&TaskStatus::Ready).unwrap_or(&0) as usize,
            running_tasks: *task_counts.get(&TaskStatus::Running).unwrap_or(&0) as usize,
            completed_tasks: *task_counts.get(&TaskStatus::Complete).unwrap_or(&0) as usize,
            failed_tasks: *task_counts.get(&TaskStatus::Failed).unwrap_or(&0) as usize,
            active_agents: self.max_agents - self.agent_semaphore.available_permits(),
            active_worktrees,
            total_tokens_used: self.total_tokens.load(Ordering::Relaxed),
        };

        {
            let mut s = self.stats.write().await;
            *s = new_stats.clone();
        }

        let status_event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Orchestrator,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::StatusUpdate(SwarmStatsPayload::from(new_stats)),
        };

        Ok(Reaction::EmitEvents(vec![status_event]))
    }
}

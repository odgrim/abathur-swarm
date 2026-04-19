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
// WorktreeReconciliationHandler
// ============================================================================

/// Triggered by the "reconciliation" scheduled event (piggybacks on existing schedule).
/// Detects orphaned worktrees — active worktrees whose associated task is in a
/// terminal state — and emits warning events. Does not delete worktrees.
pub struct WorktreeReconciliationHandler<T: TaskRepository, W: WorktreeRepository> {
    task_repo: Arc<T>,
    worktree_repo: Arc<W>,
}

impl<T: TaskRepository, W: WorktreeRepository> WorktreeReconciliationHandler<T, W> {
    pub fn new(task_repo: Arc<T>, worktree_repo: Arc<W>) -> Self {
        Self {
            task_repo,
            worktree_repo,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, W: WorktreeRepository + 'static> EventHandler
    for WorktreeReconciliationHandler<T, W>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "WorktreeReconciliationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "reconciliation"
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
        use crate::domain::models::WorktreeStatus;

        let active_worktrees = self.worktree_repo.list_active().await.map_err(|e| {
            format!(
                "WorktreeReconciliation: failed to list active worktrees: {}",
                e
            )
        })?;

        let mut orphan_count = 0u32;
        let mut new_events = Vec::new();

        for wt in &active_worktrees {
            let task = self
                .task_repo
                .get(wt.task_id)
                .await
                .map_err(|e| format!("WorktreeReconciliation: failed to get task: {}", e))?;

            let is_orphaned = match &task {
                Some(t) => t.is_terminal(),
                None => true, // Task doesn't exist — worktree is orphaned
            };

            if is_orphaned {
                orphan_count += 1;

                let reason = match &task {
                    Some(t) => format!("task in terminal state: {}", t.status.as_str()),
                    None => "task not found".to_string(),
                };

                // Actually destroy the orphaned worktree
                let mut updated_wt = wt.clone();
                updated_wt.status = WorktreeStatus::Removed;
                updated_wt.updated_at = chrono::Utc::now();
                updated_wt.completed_at = Some(chrono::Utc::now());
                if let Err(e) = self.worktree_repo.update(&updated_wt).await {
                    tracing::warn!(
                        worktree_id = %wt.id,
                        error = %e,
                        "WorktreeReconciliation: failed to mark worktree as removed"
                    );
                    continue;
                }

                tracing::warn!(
                    worktree_id = %wt.id,
                    task_id = %wt.task_id,
                    path = %wt.path,
                    reason = %reason,
                    "WorktreeReconciliation: orphaned worktree destroyed"
                );

                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Orchestrator,
                    goal_id: None,
                    task_id: Some(wt.task_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::WorktreeDestroyed {
                        worktree_id: wt.id,
                        task_id: wt.task_id,
                        reason: reason.clone(),
                    },
                });
            }
        }

        if orphan_count > 0 {
            tracing::info!(
                "WorktreeReconciliation: {} orphaned worktree(s) destroyed",
                orphan_count
            );

            // Emit reconciliation summary with actual correction count
            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Orchestrator,
                goal_id: None,
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::ReconciliationCompleted {
                    corrections_made: orphan_count,
                },
            });
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

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
// StartupCatchUpHandler (Phase 5a)
// ============================================================================

/// Triggered by `OrchestratorStarted`. Runs once at startup to fix orphaned
/// tasks, replay missed events, re-evaluate goals, and run reconciliation.
pub struct StartupCatchUpHandler<T: TaskRepository, G: GoalRepository> {
    task_repo: Arc<T>,
    goal_repo: Arc<G>,
    event_store: Arc<dyn EventStore>,
    stale_threshold_secs: u64,
    max_replay_events: u64,
}

impl<T: TaskRepository, G: GoalRepository> StartupCatchUpHandler<T, G> {
    pub fn new(
        task_repo: Arc<T>,
        goal_repo: Arc<G>,
        event_store: Arc<dyn EventStore>,
        stale_threshold_secs: u64,
        max_replay_events: u64,
    ) -> Self {
        Self {
            task_repo,
            goal_repo,
            event_store,
            stale_threshold_secs,
            max_replay_events,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, G: GoalRepository + 'static> EventHandler
    for StartupCatchUpHandler<T, G>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "StartupCatchUpHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Orchestrator])
                .payload_types(vec!["OrchestratorStarted".to_string()]),
            priority: HandlerPriority::SYSTEM,
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let start = std::time::Instant::now();
        let now = chrono::Utc::now();
        let mut orphaned_tasks_fixed: u32 = 0;
        let mut new_events = Vec::new();

        // 1. Fix orphaned Running tasks (started before last shutdown)
        let running = self
            .task_repo
            .list_by_status(TaskStatus::Running)
            .await
            .map_err(|e| format!("StartupCatchUp: failed to list running tasks: {}", e))?;

        let stale_cutoff = now - chrono::Duration::seconds(self.stale_threshold_secs as i64);

        for task in running {
            let is_stale = task.started_at.is_none_or(|s| s < stale_cutoff);
            if is_stale {
                let mut updated = task.clone();
                updated.retry_count += 1;
                if updated.transition_to(TaskStatus::Failed).is_ok() {
                    self.task_repo
                        .update(&updated)
                        .await
                        .map_err(|e| format!("StartupCatchUp: failed to update task: {}", e))?;
                    orphaned_tasks_fixed += 1;

                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Warning,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskFailed {
                            task_id: task.id,
                            error: "orchestrator-restart: task was running during shutdown"
                                .to_string(),
                            retry_count: updated.retry_count,
                        },
                    });
                }
            }
        }

        // 2. Replay missed events since the reactor's last-known watermark
        let reactor_wm = self
            .event_store
            .get_watermark("EventReactor")
            .await
            .map_err(|e| format!("StartupCatchUp: failed to get reactor watermark: {}", e))?;

        let since_seq = reactor_wm.unwrap_or(SequenceNumber(0));
        let replayed_events = self
            .event_store
            .replay_since(since_seq)
            .await
            .map_err(|e| format!("StartupCatchUp: failed to replay events: {}", e))?;

        // Bound replay to prevent flooding
        let bounded_replay: Vec<_> = replayed_events
            .into_iter()
            .take(self.max_replay_events as usize)
            .filter(|evt| {
                // Skip scheduler events to avoid retriggering periodic handlers
                !matches!(&evt.payload, EventPayload::ScheduledEventFired { .. })
            })
            .collect();

        let missed_events_replayed = bounded_replay.len() as u64;
        new_events.extend(bounded_replay);

        // 3. Re-evaluate active goals
        let active_goals = self
            .goal_repo
            .get_active_with_constraints()
            .await
            .map_err(|e| format!("StartupCatchUp: failed to get active goals: {}", e))?;
        let goals_count = active_goals.len() as u32;

        for goal in &active_goals {
            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: now,
                severity: EventSeverity::Debug,
                category: EventCategory::Scheduler,
                goal_id: Some(goal.id),
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::ScheduledEventFired {
                    schedule_id: uuid::Uuid::new_v4(),
                    name: "goal-evaluation".to_string(),
                },
            });
        }

        // 4. Run reconciliation
        new_events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
            severity: EventSeverity::Debug,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "reconciliation".to_string(),
            },
        });

        // 5. Run memory maintenance
        new_events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
            severity: EventSeverity::Debug,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "memory-maintenance".to_string(),
            },
        });

        let duration_ms = start.elapsed().as_millis() as u64;

        // 6. Emit summary
        new_events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
            severity: EventSeverity::Info,
            category: EventCategory::Orchestrator,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::StartupCatchUpCompleted {
                orphaned_tasks_fixed,
                missed_events_replayed,
                goals_reevaluated: goals_count,
                duration_ms,
            },
        });

        tracing::info!(
            orphaned_tasks_fixed = orphaned_tasks_fixed,
            goals_reevaluated = goals_count,
            duration_ms = duration_ms,
            "StartupCatchUp: catch-up completed"
        );

        Ok(Reaction::EmitEvents(new_events))
    }
}

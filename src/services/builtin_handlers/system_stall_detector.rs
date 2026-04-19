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
// SystemStallDetectorHandler
// ============================================================================

/// Monitors global task activity and fires `HumanEscalationRequired` when the
/// swarm appears idle for longer than 2× the goal convergence check interval.
///
/// "Idle" means no tasks were created or completed between successive checks.
/// The handler tracks a running snapshot of `(completed_count, failed_count, pending_count)`
/// and a `last_activity` timestamp. On each tick it queries `count_by_status`
/// and bumps `last_activity` whenever the snapshot changes or Running tasks
/// exist (which implies work is in progress).
///
/// Triggered by `ScheduledEventFired { name: "system-stall-check" }`.
pub struct SystemStallDetectorHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    /// Maximum idle time before escalation (seconds).
    stall_threshold_secs: u64,
    /// Internal state: `(last_activity, prev_completed, prev_failed, prev_pending)`.
    state: RwLock<(chrono::DateTime<chrono::Utc>, u64, u64, u64)>,
}

impl<T: TaskRepository> SystemStallDetectorHandler<T> {
    /// Create with an explicit stall threshold.
    ///
    /// The default used by the orchestrator is `2 × goal_convergence_check_interval_secs`.
    pub fn new(task_repo: Arc<T>, stall_threshold_secs: u64) -> Self {
        Self {
            task_repo,
            stall_threshold_secs,
            state: RwLock::new((chrono::Utc::now(), 0, 0, 0)),
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for SystemStallDetectorHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "SystemStallDetectorHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "system-stall-check"
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
        let name = match &event.payload {
            EventPayload::ScheduledEventFired { name, .. } => name.as_str(),
            _ => return Ok(Reaction::None),
        };

        if name != "system-stall-check" {
            return Ok(Reaction::None);
        }

        let counts = self
            .task_repo
            .count_by_status()
            .await
            .map_err(|e| format!("SystemStallDetector: failed to count tasks: {}", e))?;

        let completed = *counts.get(&TaskStatus::Complete).unwrap_or(&0);
        let failed = *counts.get(&TaskStatus::Failed).unwrap_or(&0);
        let pending = *counts.get(&TaskStatus::Pending).unwrap_or(&0);
        let running = *counts.get(&TaskStatus::Running).unwrap_or(&0);
        let ready = *counts.get(&TaskStatus::Ready).unwrap_or(&0);

        let now = chrono::Utc::now();
        let mut state = self.state.write().await;
        let (
            ref mut last_activity,
            ref mut prev_completed,
            ref mut prev_failed,
            ref mut prev_pending,
        ) = *state;

        // Activity detected if:
        // 1. Completed/failed counts changed (tasks finished since last check)
        // 2. Pending count changed (new tasks created since last check)
        // 3. Running or ready tasks exist (work is in progress)
        let snapshot_changed =
            completed != *prev_completed || failed != *prev_failed || pending != *prev_pending;
        let work_in_progress = running > 0 || ready > 0;

        if snapshot_changed || work_in_progress {
            *last_activity = now;
            *prev_completed = completed;
            *prev_failed = failed;
            *prev_pending = pending;
            return Ok(Reaction::None);
        }

        // No activity — check if we've exceeded the threshold
        let idle_secs = (now - *last_activity).num_seconds().max(0) as u64;

        if idle_secs < self.stall_threshold_secs {
            tracing::debug!(
                idle_secs,
                threshold = self.stall_threshold_secs,
                "SystemStallDetector: swarm idle but within threshold"
            );
            return Ok(Reaction::None);
        }

        tracing::warn!(
            idle_secs,
            threshold = self.stall_threshold_secs,
            "SystemStallDetector: swarm stalled, emitting escalation"
        );

        // 1. Escalation event for observability / human notification
        let escalation = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
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
                    "System stall detected: no tasks created or completed for {} seconds (threshold: {}s). Auto-recovery triggered via goal-convergence-check.",
                    idle_secs, self.stall_threshold_secs,
                ),
                urgency: "high".to_string(),
                questions: vec![
                    "The swarm has had no task activity. Are there goals that need new work generated?".to_string(),
                    "Auto-recovery has been triggered — a goal convergence check is being fired.".to_string(),
                ],
                is_blocking: false,
            }),
        };

        // 2. Synthetic convergence-check trigger for auto-recovery
        let convergence_trigger = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
            severity: EventSeverity::Info,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "goal-convergence-check".to_string(),
            },
        };

        // Reset activity timestamp so we don't fire repeatedly every tick
        *last_activity = now;

        Ok(Reaction::EmitEvents(vec![escalation, convergence_trigger]))
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use super::super::*;
    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, task_repository::SqliteTaskRepository,
    };
    use crate::domain::models::{Task, TaskStatus};
    use crate::services::EventBusConfig;
    use crate::services::task_service::TaskService;
    use std::sync::Arc;
    use uuid::Uuid;

    #[allow(dead_code)]
    async fn setup_task_repo() -> Arc<SqliteTaskRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
        Arc::new(SqliteTaskRepository::new(pool))
    }

    #[allow(dead_code)]
    fn make_task_service(
        repo: &Arc<SqliteTaskRepository>,
    ) -> Arc<TaskService<SqliteTaskRepository>> {
        Arc::new(TaskService::new(repo.clone()))
    }

    // ========================================================================

    /// Helper: build a "system-stall-check" ScheduledEventFired event.
    fn make_stall_check_event() -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: Uuid::new_v4(),
                name: "system-stall-check".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_stall_detector_ignores_wrong_schedule() {
        let repo = setup_task_repo().await;
        let handler = SystemStallDetectorHandler::new(repo, 10);

        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: Uuid::new_v4(),
                name: "stats-update".to_string(),
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_stall_detector_no_escalation_when_running_tasks_exist() {
        let repo = setup_task_repo().await;
        // Threshold of 0 means any idle period should fire — but running tasks
        // count as activity, so it should still be Reaction::None.
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // Create a Running task so the handler sees work-in-progress.
        let mut task = Task::new("Running task");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_stall_detector_no_escalation_when_snapshot_changes() {
        let repo = setup_task_repo().await;
        // Threshold of 0 — but the snapshot change resets last_activity.
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // First call: establishes initial snapshot (all zeros, no running).
        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let _ = handler.handle(&event, &ctx).await.unwrap();

        // Now create a completed task so the snapshot changes on next tick.
        let mut task = Task::new("Completed task");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&task).await.unwrap();

        let event2 = make_stall_check_event();
        let reaction = handler.handle(&event2, &ctx).await.unwrap();
        // Snapshot changed → activity reset → no escalation
        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_stall_detector_escalation_on_idle() {
        let repo = setup_task_repo().await;
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // First call: snapshot is (0,0,0), no running/ready → idle, but
        // last_activity was just set to "now" in the constructor, so we need
        // to force the internal timestamp into the past.
        {
            let mut state = handler.state.write().await;
            state.0 = chrono::Utc::now() - chrono::Duration::seconds(100);
        }

        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        match reaction {
            Reaction::EmitEvents(events) => {
                // Should emit 2 events: escalation + convergence trigger
                assert_eq!(events.len(), 2);
                assert_eq!(events[0].category, EventCategory::Escalation);
                match &events[0].payload {
                    EventPayload::HumanEscalationRequired(p) => {
                        assert!(p.reason.contains("System stall detected"));
                        assert!(p.reason.contains("Auto-recovery triggered"));
                        assert_eq!(p.urgency, "high");
                        assert!(!p.is_blocking);
                    }
                    other => panic!("Expected HumanEscalationRequired, got {:?}", other),
                }
                assert_eq!(events[1].category, EventCategory::Scheduler);
                match &events[1].payload {
                    EventPayload::ScheduledEventFired { name, .. } => {
                        assert_eq!(name, "goal-convergence-check");
                    }
                    other => panic!("Expected ScheduledEventFired, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents escalation"),
        }
    }

    #[tokio::test]
    async fn test_stall_detector_emits_both_escalation_and_convergence_trigger() {
        let repo = setup_task_repo().await;
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // Force last_activity into the past so the stall threshold is exceeded.
        {
            let mut state = handler.state.write().await;
            state.0 = chrono::Utc::now() - chrono::Duration::seconds(500);
        }

        let event = make_stall_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        let events = match reaction {
            Reaction::EmitEvents(events) => events,
            Reaction::None => panic!("Expected EmitEvents, got None"),
        };

        // Verify exactly 2 events
        assert_eq!(
            events.len(),
            2,
            "Stall detector should emit exactly 2 events"
        );

        // First event: HumanEscalationRequired
        assert_eq!(events[0].severity, EventSeverity::Warning);
        assert_eq!(events[0].category, EventCategory::Escalation);
        match &events[0].payload {
            EventPayload::HumanEscalationRequired(p) => {
                assert!(
                    p.reason.contains("System stall detected"),
                    "Reason should mention stall: {}",
                    p.reason
                );
                assert!(
                    p.reason.contains("Auto-recovery triggered"),
                    "Reason should mention auto-recovery: {}",
                    p.reason
                );
                assert_eq!(p.urgency, "high");
                assert!(!p.is_blocking);
            }
            other => panic!("Event 0: expected HumanEscalationRequired, got {:?}", other),
        }

        // Second event: ScheduledEventFired for goal-convergence-check
        assert_eq!(events[1].severity, EventSeverity::Info);
        assert_eq!(events[1].category, EventCategory::Scheduler);
        match &events[1].payload {
            EventPayload::ScheduledEventFired { name, schedule_id } => {
                assert_eq!(name, "goal-convergence-check");
                // schedule_id should be a fresh UUID (non-nil)
                assert_ne!(*schedule_id, uuid::Uuid::nil());
            }
            other => panic!("Event 1: expected ScheduledEventFired, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_stall_detector_resets_after_escalation() {
        let repo = setup_task_repo().await;
        // Threshold = 0 so any idle fires immediately.
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // Force last_activity into the past.
        {
            let mut state = handler.state.write().await;
            state.0 = chrono::Utc::now() - chrono::Duration::seconds(100);
        }

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // First call should escalate.
        let event1 = make_stall_check_event();
        let reaction1 = handler.handle(&event1, &ctx).await.unwrap();
        assert!(matches!(reaction1, Reaction::EmitEvents(_)));

        // Second call immediately after: last_activity was reset by the
        // escalation, so now the idle_secs is ~0, which is not >= threshold(0)
        // *only if* there is zero elapsed time. In practice the handler uses
        // `< threshold` (strict), and 0 < 0 is false, so threshold=0 will
        // always fire unless activity is detected. Let's use threshold=1 to
        // make this test reliable.
        let handler2 = SystemStallDetectorHandler::new(repo.clone(), 1);
        {
            let mut state = handler2.state.write().await;
            state.0 = chrono::Utc::now() - chrono::Duration::seconds(100);
        }
        // First call escalates
        let reaction_a = handler2
            .handle(&make_stall_check_event(), &ctx)
            .await
            .unwrap();
        assert!(matches!(reaction_a, Reaction::EmitEvents(_)));
        // Second call immediately: last_activity was just reset, idle < 1s
        let reaction_b = handler2
            .handle(&make_stall_check_event(), &ctx)
            .await
            .unwrap();
        assert!(matches!(reaction_b, Reaction::None));
    }
}

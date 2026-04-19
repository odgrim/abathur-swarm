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

/// Slow-path reconciliation handler (LOW priority, every ~5min).
///
/// Handles expensive checks only: stale Running tasks, Validating timeouts,
/// workflow parking timeouts. Cheap state transition checks (Pending→Ready,
/// Blocked→Ready) have been moved to `FastReconciliationHandler`.
///
/// Triggered by the "reconciliation" scheduled event.
pub struct ReconciliationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    /// Tasks stuck in Running longer than this are considered stale (seconds).
    stale_task_timeout_secs: u64,
    /// Tasks stuck in Validating longer than this are considered stale (seconds).
    /// Shorter than Running timeout because Validating should resolve quickly.
    stale_validating_timeout_secs: u64,
}

impl<T: TaskRepository> ReconciliationHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self {
            task_repo,
            stale_task_timeout_secs: 7200,       // 2 hours default
            stale_validating_timeout_secs: 1800, // 30 minutes default
        }
    }

    pub fn with_stale_timeout(mut self, secs: u64) -> Self {
        self.stale_task_timeout_secs = secs;
        self
    }

    pub fn with_stale_validating_timeout(mut self, secs: u64) -> Self {
        self.stale_validating_timeout_secs = secs;
        self
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for ReconciliationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ReconciliationHandler".to_string(),
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
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let mut corrections: u32 = 0;
        let mut new_events = Vec::new();
        let mut processed_ids: HashSet<uuid::Uuid> = HashSet::new();

        // NOTE: Pending→Ready and Blocked→Ready checks have been moved to
        // FastReconciliationHandler (NORMAL priority, every ~15s) for faster
        // state transition recovery. This handler only runs expensive checks.

        // Stale-task detection: tasks stuck in Running for > stale_task_timeout_secs
        // Tiered warnings: 50% -> TaskRunningLong, 80% -> TaskRunningCritical + escalation, 100% -> fail
        let running = self
            .task_repo
            .list_by_status(TaskStatus::Running)
            .await
            .map_err(|e| format!("Failed to list running tasks: {}", e))?;

        let now = chrono::Utc::now();
        let timeout = chrono::Duration::seconds(self.stale_task_timeout_secs as i64);
        let warning_threshold =
            chrono::Duration::seconds((self.stale_task_timeout_secs as f64 * 0.5) as i64);
        let critical_threshold =
            chrono::Duration::seconds((self.stale_task_timeout_secs as f64 * 0.8) as i64);

        for task in &running {
            if let Some(started_at) = task.started_at {
                let elapsed = now - started_at;
                let runtime_secs = elapsed.num_seconds().max(0) as u64;

                if elapsed > timeout {
                    // 100% — fail the task
                    let mut updated = task.clone();
                    updated.retry_count += 1;
                    if updated.transition_to(TaskStatus::Failed).is_ok()
                        && try_update_task(&*self.task_repo, &updated, "running stale->failed")
                            .await?
                    {
                        corrections += 1;
                        processed_ids.insert(task.id);

                        new_events.push(UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: chrono::Utc::now(),
                            severity: EventSeverity::Warning,
                            category: EventCategory::Task,
                            goal_id: None,
                            task_id: Some(task.id),
                            correlation_id: event.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::TaskFailed {
                                task_id: task.id,
                                error: format!(
                                    "stale-timeout: task running for > {}s",
                                    self.stale_task_timeout_secs
                                ),
                                retry_count: updated.retry_count,
                            },
                        });

                        tracing::warn!(
                            "ReconciliationHandler: stale task {} failed after {}s (started: {})",
                            task.id,
                            self.stale_task_timeout_secs,
                            started_at
                        );
                    }
                } else if elapsed > critical_threshold {
                    // 80% — critical warning + escalation
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
                        payload: EventPayload::TaskRunningCritical {
                            task_id: task.id,
                            runtime_secs,
                        },
                    });

                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Warning,
                        category: EventCategory::Escalation,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::HumanEscalationNeeded(HumanEscalationPayload {
                            goal_id: None,
                            task_id: Some(task.id),
                            reason: format!(
                                "Task '{}' running for {}s (80% of {}s timeout)",
                                task.title, runtime_secs, self.stale_task_timeout_secs
                            ),
                            urgency: "high".to_string(),
                            questions: vec![],
                            is_blocking: false,
                        }),
                    });
                } else if elapsed > warning_threshold {
                    // 50% — early warning
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Info,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskRunningLong {
                            task_id: task.id,
                            runtime_secs,
                        },
                    });
                }
            }
        }

        // Stale-task detection: tasks stuck in Validating for > stale_validating_timeout_secs
        // Uses updated_at (when the task entered Validating) instead of started_at
        // (when the agent first spawned), since Validating should resolve quickly.
        let validating = self
            .task_repo
            .list_by_status(TaskStatus::Validating)
            .await
            .map_err(|e| format!("Failed to list validating tasks: {}", e))?;

        let validating_timeout =
            chrono::Duration::seconds(self.stale_validating_timeout_secs as i64);
        let validating_warning =
            chrono::Duration::seconds((self.stale_validating_timeout_secs as f64 * 0.5) as i64);
        let validating_critical =
            chrono::Duration::seconds((self.stale_validating_timeout_secs as f64 * 0.8) as i64);

        for task in &validating {
            let elapsed = now - task.updated_at;
            let validating_secs = elapsed.num_seconds().max(0) as u64;

            if elapsed > validating_timeout {
                // 100% — workflow-aware action
                let ws = task.workflow_state();

                match ws {
                    Some(WorkflowState::Verifying {
                        phase_index,
                        ref phase_name,
                        retry_count,
                        ..
                    }) => {
                        // Re-trigger verification instead of failing
                        tracing::warn!(
                            "ReconciliationHandler: stale validating task {} has WorkflowState::Verifying — re-triggering verification after {}s (updated_at: {})",
                            task.id,
                            validating_secs,
                            task.updated_at
                        );

                        // Clear verification idempotency keys so the handler doesn't dedup
                        let mut updated = task.clone();
                        updated.clear_verification_retry_count();
                        updated.clear_verification_idempotency_key();
                        updated.updated_at = chrono::Utc::now();
                        let _ = try_update_task(
                            &*self.task_repo,
                            &updated,
                            "validating stale: clear verification keys for retry",
                        )
                        .await;

                        new_events.push(UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: chrono::Utc::now(),
                            severity: EventSeverity::Warning,
                            category: EventCategory::Workflow,
                            goal_id: None,
                            task_id: Some(task.id),
                            correlation_id: event.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::WorkflowVerificationRequested {
                                task_id: task.id,
                                phase_index,
                                phase_name: phase_name.clone(),
                                retry_count,
                            },
                        });
                    }

                    Some(WorkflowState::PhaseReady {
                        ref workflow_name, ..
                    }) => {
                        // Inconsistent state: Validating + PhaseReady is a deadlock
                        let mut updated = task.clone();
                        updated.retry_count += 1;
                        let failed_ws = WorkflowState::Failed {
                            workflow_name: workflow_name.clone(),
                            error: "Stale validation timeout: state inconsistency (Validating+PhaseReady)".to_string(),
                        };
                        let _ = updated.set_workflow_state(&failed_ws);
                        if updated.transition_to(TaskStatus::Failed).is_ok()
                            && try_update_task(
                                &*self.task_repo,
                                &updated,
                                "validating stale: state inconsistency (Validating+PhaseReady)",
                            )
                            .await?
                        {
                            corrections += 1;

                            new_events.push(UnifiedEvent {
                                id: EventId::new(),
                                sequence: SequenceNumber(0),
                                timestamp: chrono::Utc::now(),
                                severity: EventSeverity::Warning,
                                category: EventCategory::Task,
                                goal_id: None,
                                task_id: Some(task.id),
                                correlation_id: event.correlation_id,
                                source_process_id: None,
                                payload: EventPayload::TaskFailed {
                                    task_id: task.id,
                                    error: "Validation timed out: state inconsistency (Validating+PhaseReady)".to_string(),
                                    retry_count: updated.retry_count,
                                },
                            });

                            tracing::warn!(
                                "ReconciliationHandler: stale validating task {} failed after {}s — state inconsistency (Validating+PhaseReady)",
                                task.id,
                                validating_secs
                            );
                        }
                    }

                    Some(ref ws) if ws.is_terminal() => {
                        // Terminal workflow state but task still Validating — align task status
                        let target_status = match ws {
                            WorkflowState::Completed { .. } => TaskStatus::Complete,
                            WorkflowState::Failed { .. } | WorkflowState::Rejected { .. } => {
                                TaskStatus::Failed
                            }
                            _ => TaskStatus::Failed, // shouldn't happen, but safe fallback
                        };

                        let mut updated = task.clone();
                        let transition_ok = if target_status == TaskStatus::Complete {
                            updated.transition_to(TaskStatus::Complete).is_ok()
                        } else {
                            updated.retry_count += 1;
                            updated.transition_to(TaskStatus::Failed).is_ok()
                        };

                        if transition_ok
                            && try_update_task(
                                &*self.task_repo,
                                &updated,
                                &format!(
                                    "validating stale: terminal workflow_state -> {:?}",
                                    target_status
                                ),
                            )
                            .await?
                        {
                            corrections += 1;

                            let payload = if target_status == TaskStatus::Complete {
                                EventPayload::TaskCompleted {
                                    task_id: task.id,
                                    tokens_used: 0,
                                }
                            } else {
                                EventPayload::TaskFailed {
                                    task_id: task.id,
                                    error: format!(
                                        "stale-timeout: terminal workflow_state ({:?}) but task stuck in Validating for > {}s",
                                        ws, self.stale_validating_timeout_secs
                                    ),
                                    retry_count: updated.retry_count,
                                }
                            };

                            new_events.push(UnifiedEvent {
                                id: EventId::new(),
                                sequence: SequenceNumber(0),
                                timestamp: chrono::Utc::now(),
                                severity: EventSeverity::Warning,
                                category: EventCategory::Task,
                                goal_id: None,
                                task_id: Some(task.id),
                                correlation_id: event.correlation_id,
                                source_process_id: None,
                                payload,
                            });

                            tracing::warn!(
                                "ReconciliationHandler: stale validating task {} aligned to {:?} after {}s — terminal workflow_state {:?}",
                                task.id,
                                target_status,
                                validating_secs,
                                ws
                            );
                        }
                    }

                    None => {
                        // No workflow state (standalone task) — fail as before
                        let mut updated = task.clone();
                        updated.retry_count += 1;
                        if updated.transition_to(TaskStatus::Failed).is_ok()
                            && try_update_task(
                                &*self.task_repo,
                                &updated,
                                "validating stale->failed",
                            )
                            .await?
                        {
                            corrections += 1;

                            new_events.push(UnifiedEvent {
                                id: EventId::new(),
                                sequence: SequenceNumber(0),
                                timestamp: chrono::Utc::now(),
                                severity: EventSeverity::Warning,
                                category: EventCategory::Task,
                                goal_id: None,
                                task_id: Some(task.id),
                                correlation_id: event.correlation_id,
                                source_process_id: None,
                                payload: EventPayload::TaskFailed {
                                    task_id: task.id,
                                    error: format!(
                                        "stale-timeout: task validating for > {}s",
                                        self.stale_validating_timeout_secs
                                    ),
                                    retry_count: updated.retry_count,
                                },
                            });

                            tracing::warn!(
                                "ReconciliationHandler: stale validating task {} failed after {}s (standalone, no workflow_state)",
                                task.id,
                                validating_secs
                            );
                        }
                    }

                    Some(ref other_ws) => {
                        // Other non-terminal workflow states (e.g. PhaseRunning, FanningOut, etc.) — fail
                        let mut updated = task.clone();
                        updated.retry_count += 1;
                        let failed_ws = WorkflowState::Failed {
                            workflow_name: other_ws.workflow_name().to_string(),
                            error: format!(
                                "Stale validation timeout: task validating for > {}s",
                                self.stale_validating_timeout_secs
                            ),
                        };
                        let _ = updated.set_workflow_state(&failed_ws);
                        if updated.transition_to(TaskStatus::Failed).is_ok()
                            && try_update_task(
                                &*self.task_repo,
                                &updated,
                                "validating stale->failed",
                            )
                            .await?
                        {
                            corrections += 1;

                            new_events.push(UnifiedEvent {
                                id: EventId::new(),
                                sequence: SequenceNumber(0),
                                timestamp: chrono::Utc::now(),
                                severity: EventSeverity::Warning,
                                category: EventCategory::Task,
                                goal_id: None,
                                task_id: Some(task.id),
                                correlation_id: event.correlation_id,
                                source_process_id: None,
                                payload: EventPayload::TaskFailed {
                                    task_id: task.id,
                                    error: format!(
                                        "stale-timeout: task validating for > {}s",
                                        self.stale_validating_timeout_secs
                                    ),
                                    retry_count: updated.retry_count,
                                },
                            });

                            tracing::warn!(
                                "ReconciliationHandler: stale validating task {} failed after {}s (updated_at: {})",
                                task.id,
                                validating_secs,
                                task.updated_at
                            );
                        }
                    }
                }
            } else if elapsed > validating_critical {
                // 80% — critical warning + escalation
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
                    payload: EventPayload::TaskRunningCritical {
                        task_id: task.id,
                        runtime_secs: validating_secs,
                    },
                });

                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: now,
                    severity: EventSeverity::Warning,
                    category: EventCategory::Escalation,
                    goal_id: None,
                    task_id: Some(task.id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::HumanEscalationNeeded(HumanEscalationPayload {
                        goal_id: None,
                        task_id: Some(task.id),
                        reason: format!(
                            "Task '{}' validating for {}s (80% of {}s timeout)",
                            task.title, validating_secs, self.stale_validating_timeout_secs
                        ),
                        urgency: "high".to_string(),
                        questions: vec![],
                        is_blocking: false,
                    }),
                });
            } else if elapsed > validating_warning {
                // 50% — early warning
                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: now,
                    severity: EventSeverity::Info,
                    category: EventCategory::Task,
                    goal_id: None,
                    task_id: Some(task.id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskRunningLong {
                        task_id: task.id,
                        runtime_secs: validating_secs,
                    },
                });
            }
        }

        // Workflow parking state timeout detection: Running tasks whose workflow_state
        // is PhaseReady, PhaseGate, or Verifying indicate the overmind has stalled.
        // Use half the stale timeout to catch these before the full Running staleness fires.
        let parking_timeout = chrono::Duration::seconds((self.stale_task_timeout_secs / 2) as i64);
        for task in &running {
            // Skip tasks already processed by the stale-running loop above
            if processed_ids.contains(&task.id) {
                continue;
            }
            if let Some(started_at) = task.started_at {
                let elapsed = now - started_at;
                if elapsed > parking_timeout {
                    // Check if this task has a workflow state that indicates parking
                    let ws = task.workflow_state();
                    if let Some(ws) = ws {
                        let is_parked = matches!(
                            ws,
                            WorkflowState::PhaseReady { .. }
                                | WorkflowState::PhaseGate { .. }
                                | WorkflowState::Verifying { .. }
                        );
                        if is_parked {
                            let runtime_secs = elapsed.num_seconds().max(0) as u64;
                            let workflow_name = ws.workflow_name().to_string();
                            let error_msg = format!(
                                "workflow-parking-timeout: task parked in {:?} for {}s (limit: {}s)",
                                std::mem::discriminant(&ws),
                                runtime_secs,
                                self.stale_task_timeout_secs / 2,
                            );

                            // Write WorkflowState::Failed
                            let failed_ws = WorkflowState::Failed {
                                workflow_name,
                                error: error_msg.clone(),
                            };
                            let mut updated = task.clone();
                            let _ = updated.set_workflow_state(&failed_ws);
                            updated.retry_count += 1;
                            if updated.transition_to(TaskStatus::Failed).is_ok()
                                && try_update_task(
                                    &*self.task_repo,
                                    &updated,
                                    "parking stale->failed",
                                )
                                .await?
                            {
                                corrections += 1;

                                new_events.push(UnifiedEvent {
                                    id: EventId::new(),
                                    sequence: SequenceNumber(0),
                                    timestamp: chrono::Utc::now(),
                                    severity: EventSeverity::Warning,
                                    category: EventCategory::Task,
                                    goal_id: None,
                                    task_id: Some(task.id),
                                    correlation_id: event.correlation_id,
                                    source_process_id: None,
                                    payload: EventPayload::TaskFailed {
                                        task_id: task.id,
                                        error: error_msg.clone(),
                                        retry_count: updated.retry_count,
                                    },
                                });

                                tracing::warn!(
                                    "ReconciliationHandler: workflow-parked task {} failed after {}s",
                                    task.id,
                                    runtime_secs
                                );
                            }
                        }
                    }
                }
            }
        }

        // Emit reconciliation completed event
        new_events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Orchestrator,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::ReconciliationCompleted {
                corrections_made: corrections,
            },
        });

        if corrections > 0 {
            tracing::info!("ReconciliationHandler: made {} corrections", corrections);
        }

        Ok(Reaction::EmitEvents(new_events))
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use super::super::*;
    use super::*;

    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, task_repository::SqliteTaskRepository,
    };
    use crate::domain::models::workflow_state::WorkflowState;
    use crate::domain::models::{Task, TaskStatus};
    use std::sync::Arc;

    #[allow(dead_code)]
    async fn setup_task_repo() -> Arc<SqliteTaskRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
        Arc::new(SqliteTaskRepository::new(pool))
    }

    fn make_reconciliation_event() -> UnifiedEvent {
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
                schedule_id: uuid::Uuid::new_v4(),
                name: "reconciliation".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_reconciliation_stale_validating_task_fails() {
        let repo = setup_task_repo().await;
        // Use a 100s validating timeout so anything older than 100s gets failed
        let handler = ReconciliationHandler::new(repo.clone()).with_stale_validating_timeout(100);

        // Create a task stuck in Validating for longer than the timeout
        let mut task = Task::new("Stuck validating task");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Validating).unwrap();
        // Set updated_at to 200 seconds ago (well past 100s timeout)
        task.updated_at = chrono::Utc::now() - chrono::Duration::seconds(200);
        repo.create(&task).await.unwrap();

        let event = make_reconciliation_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Verify the task was failed
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(
            updated.status,
            TaskStatus::Failed,
            "Stale validating task should be failed"
        );

        // Verify a TaskFailed event was emitted
        match reaction {
            Reaction::EmitEvents(events) => {
                let has_task_failed = events.iter().any(|e| matches!(&e.payload, EventPayload::TaskFailed { task_id, .. } if *task_id == task.id));
                assert!(
                    has_task_failed,
                    "Should emit TaskFailed event for stale validating task"
                );
            }
            Reaction::None => panic!("Expected EmitEvents reaction"),
        }
    }

    #[tokio::test]
    async fn test_reconciliation_workflow_parking_timeout() {
        let repo = setup_task_repo().await;
        // Use a 100s timeout; parking timeout is half = 50s
        let handler = ReconciliationHandler::new(repo.clone()).with_stale_timeout(100);

        // Create a Running task with a workflow_state of PhaseReady (parked)
        let mut task = Task::new("Parked workflow task");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        // Set started_at to 60 seconds ago (past the 50s parking timeout but under 100s stale timeout)
        task.started_at = Some(chrono::Utc::now() - chrono::Duration::seconds(60));
        // Set workflow_state to PhaseReady (a parking state)
        let parked_state = WorkflowState::PhaseReady {
            workflow_name: "code".to_string(),
            phase_index: 1,
            phase_name: "implement".to_string(),
        };
        task.set_workflow_state(&parked_state).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_reconciliation_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Verify the task was failed
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(
            updated.status,
            TaskStatus::Failed,
            "Workflow-parked task should be failed"
        );

        // Verify workflow_state was set to Failed
        let ws = updated.workflow_state().expect("workflow_state present");
        assert!(
            matches!(ws, WorkflowState::Failed { .. }),
            "workflow_state should be Failed"
        );

        // Verify a TaskFailed event was emitted
        match reaction {
            Reaction::EmitEvents(events) => {
                let has_task_failed = events.iter().any(|e| matches!(&e.payload, EventPayload::TaskFailed { task_id, .. } if *task_id == task.id));
                assert!(
                    has_task_failed,
                    "Should emit TaskFailed event for parked workflow task"
                );
            }
            Reaction::None => panic!("Expected EmitEvents reaction"),
        }
    }
}

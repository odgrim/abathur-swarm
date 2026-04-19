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
// TaskSLAEnforcementHandler (Phase 2a)
// ============================================================================

/// Triggered by the "sla-check" scheduled event (60s). Queries tasks with
/// deadlines and emits tiered SLA events (warning/critical/breached).
pub struct TaskSLAEnforcementHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    warning_threshold_pct: f64,
    critical_threshold_pct: f64,
    auto_escalate_on_breach: bool,
}

impl<T: TaskRepository> TaskSLAEnforcementHandler<T> {
    pub fn new(
        task_repo: Arc<T>,
        warning_threshold_pct: f64,
        critical_threshold_pct: f64,
        auto_escalate_on_breach: bool,
    ) -> Self {
        Self {
            task_repo,
            warning_threshold_pct,
            critical_threshold_pct,
            auto_escalate_on_breach,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for TaskSLAEnforcementHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskSLAEnforcementHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "sla-check"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let now = chrono::Utc::now();
        let mut new_events = Vec::new();

        // Check all active statuses for tasks with deadlines
        for status in &[TaskStatus::Pending, TaskStatus::Ready, TaskStatus::Running] {
            let tasks = self
                .task_repo
                .list_by_status(*status)
                .await
                .map_err(|e| format!("SLA check failed: {}", e))?;

            for task in tasks {
                let deadline = match task.deadline {
                    Some(d) => d,
                    None => continue,
                };

                let total_duration = (deadline - task.created_at).num_seconds().max(1) as f64;
                let remaining = (deadline - now).num_seconds();

                if remaining <= 0 {
                    // Breached
                    let overdue_secs = -remaining;
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Critical,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskSLABreached {
                            task_id: task.id,
                            deadline: deadline.to_rfc3339(),
                            overdue_secs,
                        },
                    });

                    if self.auto_escalate_on_breach {
                        new_events.push(UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: now,
                            severity: EventSeverity::Critical,
                            category: EventCategory::Escalation,
                            goal_id: None,
                            task_id: Some(task.id),
                            correlation_id: event.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::HumanEscalationRequired(
                                HumanEscalationPayload {
                                    goal_id: None,
                                    task_id: Some(task.id),
                                    reason: format!(
                                        "Task '{}' SLA breached: overdue by {}s",
                                        task.title, overdue_secs
                                    ),
                                    urgency: "critical".to_string(),
                                    questions: vec![format!(
                                        "Task '{}' has missed its deadline. What should be done?",
                                        task.title
                                    )],
                                    is_blocking: false,
                                },
                            ),
                        });
                    }
                } else {
                    let remaining_pct = remaining as f64 / total_duration;

                    if remaining_pct < self.critical_threshold_pct {
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
                            payload: EventPayload::TaskSLACritical {
                                task_id: task.id,
                                deadline: deadline.to_rfc3339(),
                                remaining_secs: remaining,
                            },
                        });
                    } else if remaining_pct < self.warning_threshold_pct {
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
                            payload: EventPayload::TaskSLAWarning {
                                task_id: task.id,
                                deadline: deadline.to_rfc3339(),
                                remaining_secs: remaining,
                            },
                        });
                    }
                }
            }
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

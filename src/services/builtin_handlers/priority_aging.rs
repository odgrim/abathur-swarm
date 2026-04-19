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
// PriorityAgingHandler (Phase 2b)
// ============================================================================

/// Triggered by the "priority-aging" scheduled event (300s, opt-in).
/// Ages task priorities based on wait time since creation.
pub struct PriorityAgingHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    low_to_normal_secs: u64,
    normal_to_high_secs: u64,
    high_to_critical_secs: u64,
}

impl<T: TaskRepository> PriorityAgingHandler<T> {
    pub fn new(
        task_repo: Arc<T>,
        low_to_normal_secs: u64,
        normal_to_high_secs: u64,
        high_to_critical_secs: u64,
    ) -> Self {
        Self {
            task_repo,
            low_to_normal_secs,
            normal_to_high_secs,
            high_to_critical_secs,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for PriorityAgingHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "PriorityAgingHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "priority-aging"
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
        use crate::domain::models::TaskPriority;

        let now = chrono::Utc::now();
        let mut new_events = Vec::new();

        for status in &[TaskStatus::Pending, TaskStatus::Ready] {
            let tasks = self
                .task_repo
                .list_by_status(*status)
                .await
                .map_err(|e| format!("Priority aging failed: {}", e))?;

            for task in tasks {
                let wait_secs = (now - task.created_at).num_seconds() as u64;

                let new_priority = match task.priority {
                    TaskPriority::Low if wait_secs > self.low_to_normal_secs => {
                        Some(TaskPriority::Normal)
                    }
                    TaskPriority::Normal if wait_secs > self.normal_to_high_secs => {
                        Some(TaskPriority::High)
                    }
                    TaskPriority::High if wait_secs > self.high_to_critical_secs => {
                        Some(TaskPriority::Critical)
                    }
                    _ => None,
                };

                if let Some(new_pri) = new_priority {
                    let from = task.priority.as_str().to_string();
                    let to = new_pri.as_str().to_string();

                    let mut updated = task.clone();
                    updated.priority = new_pri;
                    updated.updated_at = now;
                    self.task_repo
                        .update(&updated)
                        .await
                        .map_err(|e| format!("Failed to update priority: {}", e))?;

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
                        payload: EventPayload::TaskPriorityChanged {
                            task_id: task.id,
                            from,
                            to,
                            reason: format!("priority-aging: waited {}s", wait_secs),
                        },
                    });
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

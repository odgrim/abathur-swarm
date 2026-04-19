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
// GoalReconciliationHandler
// ============================================================================

/// Periodic safety-net for goal subsystem: re-evaluates active goals,
/// detects stale ones (no recent events), logs their status, and emits
/// escalation events for goals with no activity beyond a configurable
/// threshold (default 48h).
///
/// Triggered by `ScheduledEventFired { name: "goal-reconciliation" }`.
pub struct GoalReconciliationHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    /// Hours of inactivity after which a goal triggers a human escalation.
    escalation_threshold_hours: i64,
}

impl<G: GoalRepository> GoalReconciliationHandler<G> {
    pub fn new(goal_repo: Arc<G>) -> Self {
        Self {
            goal_repo,
            escalation_threshold_hours: 48,
        }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for GoalReconciliationHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalReconciliationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "goal-reconciliation"
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

        if name != "goal-reconciliation" {
            return Ok(Reaction::None);
        }

        let active_goals = self
            .goal_repo
            .get_active_with_constraints()
            .await
            .map_err(|e| format!("GoalReconciliation: failed to get goals: {}", e))?;

        let now = chrono::Utc::now();
        let stale_threshold = chrono::Duration::hours(24);
        let escalation_threshold = chrono::Duration::hours(self.escalation_threshold_hours);
        let mut new_events = Vec::new();

        for goal in &active_goals {
            let age = now - goal.updated_at;
            if age > escalation_threshold {
                tracing::warn!(
                    goal_id = %goal.id,
                    goal_name = %goal.name,
                    hours_stale = age.num_hours(),
                    "GoalReconciliation: goal stale beyond escalation threshold, emitting escalation"
                );

                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Escalation,
                    goal_id: Some(goal.id),
                    task_id: None,
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::HumanEscalationRequired(HumanEscalationPayload {
                        goal_id: Some(goal.id),
                        task_id: None,
                        reason: format!(
                            "Goal '{}' has had no activity for {} hours",
                            goal.name,
                            age.num_hours()
                        ),
                        urgency: "medium".to_string(),
                        questions: vec![format!(
                            "Goal '{}' appears stale. Should it be continued, paused, or retired?",
                            goal.name
                        )],
                        is_blocking: false,
                    }),
                });
            } else if age > stale_threshold {
                tracing::info!(
                    goal_id = %goal.id,
                    goal_name = %goal.name,
                    hours_stale = age.num_hours(),
                    "GoalReconciliation: goal has not been updated recently"
                );
            }
        }

        tracing::debug!(
            active_goals = active_goals.len(),
            "GoalReconciliation: reconciliation sweep complete"
        );

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

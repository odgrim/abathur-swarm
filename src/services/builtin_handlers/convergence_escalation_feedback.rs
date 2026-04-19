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
// ConvergenceEscalationFeedbackHandler
// ============================================================================

/// When a human responds to an escalation during convergence, process the
/// response and feed it back into the convergence loop via a trajectory
/// specification amendment.
///
/// On receiving a `HumanResponseReceived` event:
/// 1. Check that the event has a `task_id` set on the envelope.
/// 2. Load the task and verify it has a `trajectory_id` (convergent task).
/// 3. Load the trajectory from the trajectory store.
/// 4. Amend the specification with the human's decision text.
/// 5. Persist the updated trajectory.
/// 6. If `allows_continuation` is false, add a `convergence:force_stop` hint
///    to the task context so the convergence loop can halt gracefully.
pub struct ConvergenceEscalationFeedbackHandler<T: TaskRepository, Tr: TrajectoryRepository> {
    task_repo: Arc<T>,
    trajectory_repo: Arc<Tr>,
}

impl<T: TaskRepository, Tr: TrajectoryRepository> ConvergenceEscalationFeedbackHandler<T, Tr> {
    pub fn new(task_repo: Arc<T>, trajectory_repo: Arc<Tr>) -> Self {
        Self {
            task_repo,
            trajectory_repo,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, Tr: TrajectoryRepository + 'static> EventHandler
    for ConvergenceEscalationFeedbackHandler<T, Tr>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ConvergenceEscalationFeedbackHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Escalation])
                .payload_types(vec!["HumanResponseReceived".to_string()]),
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
        let (escalation_id, decision, allows_continuation) = match &event.payload {
            EventPayload::HumanResponseReceived {
                escalation_id,
                decision,
                allows_continuation,
            } => (*escalation_id, decision.clone(), *allows_continuation),
            _ => return Ok(Reaction::None),
        };

        // Step 1: The event must have a task_id on the envelope
        let task_id = match event.task_id {
            Some(id) => id,
            None => return Ok(Reaction::None),
        };

        // Step 2: Load the task and check for a trajectory_id
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        let trajectory_id = match task.trajectory_id {
            Some(id) => id,
            None => return Ok(Reaction::None), // Not a convergent task
        };

        // Step 3: Load the trajectory
        let trajectory = self
            .trajectory_repo
            .get(&trajectory_id.to_string())
            .await
            .map_err(|e| format!("Failed to get trajectory: {}", e))?;
        let mut trajectory = match trajectory {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        // Step 4: Amend the specification with the human's decision
        let amendment = SpecificationAmendment::new(
            AmendmentSource::UserHint,
            decision.clone(),
            format!(
                "Human escalation response (escalation_id={})",
                escalation_id
            ),
        );
        trajectory.specification.add_amendment(amendment);

        // Step 5: Persist the updated trajectory
        self.trajectory_repo
            .save(&trajectory)
            .await
            .map_err(|e| format!("Failed to save trajectory: {}", e))?;

        // Step 6: If continuation is not allowed, add force_stop hint
        if !allows_continuation {
            let mut updated_task = task.clone();
            if !updated_task
                .context
                .hints
                .iter()
                .any(|h| h == "convergence:force_stop")
            {
                updated_task
                    .context
                    .hints
                    .push("convergence:force_stop".to_string());
                updated_task.updated_at = chrono::Utc::now();
                self.task_repo
                    .update(&updated_task)
                    .await
                    .map_err(|e| format!("Failed to update task with force_stop hint: {}", e))?;
            }
        }

        // Step 7: Log the feedback application
        tracing::info!(
            task_id = %task_id,
            escalation_id = %escalation_id,
            allows_continuation = allows_continuation,
            "Applied human escalation feedback as specification amendment to convergent trajectory"
        );

        Ok(Reaction::None)
    }
}

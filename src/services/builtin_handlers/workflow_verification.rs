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
// WorkflowVerificationHandler
// ============================================================================

/// Listens for `WorkflowVerificationRequested` events and runs LLM-based
/// intent verification on the completed phase subtasks. Maps the result
/// back through `WorkflowEngine::handle_verification_result()`.
pub struct WorkflowVerificationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    event_bus: Arc<EventBus>,
    intent_verifier: Arc<
        dyn crate::services::swarm_orchestrator::convergent_execution::ConvergentIntentVerifier,
    >,
    verification_enabled: bool,
}

impl<T: TaskRepository> WorkflowVerificationHandler<T> {
    pub fn new(
        task_repo: Arc<T>,
        event_bus: Arc<EventBus>,
        intent_verifier: Arc<
            dyn crate::services::swarm_orchestrator::convergent_execution::ConvergentIntentVerifier,
        >,
        verification_enabled: bool,
    ) -> Self {
        Self {
            task_repo,
            event_bus,
            intent_verifier,
            verification_enabled,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for WorkflowVerificationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "WorkflowVerificationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Workflow])
                .payload_types(vec!["WorkflowVerificationRequested".to_string()]),
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let (task_id, phase_name, retry_count) = match &event.payload {
            EventPayload::WorkflowVerificationRequested {
                task_id,
                phase_name,
                retry_count,
                ..
            } => (*task_id, phase_name.clone(), *retry_count),
            _ => return Ok(Reaction::None),
        };

        // Idempotency guard: skip if verification already dispatched for this
        // task + phase + retry combination.
        let idem_key = format!("wf-verify:{}:{}:{}", task_id, phase_name, retry_count);
        match self.task_repo.get_by_idempotency_key(&idem_key).await {
            Ok(Some(_)) => {
                tracing::debug!(
                    task_id = %task_id,
                    phase = %phase_name,
                    retry_count,
                    "WorkflowVerificationHandler: verification already dispatched (idempotency dedup)"
                );
                return Ok(Reaction::None);
            }
            Ok(None) => {} // no duplicate — proceed
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id,
                    error = %e,
                    "WorkflowVerificationHandler: idempotency check failed, proceeding anyway"
                );
            }
        }

        // Load parent task
        let mut parent_task = match self.task_repo.get(task_id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                tracing::warn!(task_id = %task_id, "WorkflowVerificationHandler: parent task not found");
                return Ok(Reaction::None);
            }
            Err(e) => {
                tracing::warn!(task_id = %task_id, error = %e, "WorkflowVerificationHandler: failed to load parent task");
                return Ok(Reaction::None);
            }
        };

        // Enrich parent task with phase context before verification.
        // This gives the verifier knowledge of which workflow phase just completed.
        {
            let workflow_state = parent_task.workflow_state();

            if let Some(ref ws) = workflow_state {
                let phase_index = ws.phase_index().unwrap_or(0);
                let total_phases_hint = parent_task
                    .context
                    .custom
                    .get("workflow_total_phases")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let phase_context = if total_phases_hint > 0 {
                    format!(
                        "workflow_phase: {} (phase {}/{})",
                        phase_name,
                        phase_index + 1,
                        total_phases_hint
                    )
                } else {
                    format!("workflow_phase: {} (phase {})", phase_name, phase_index + 1)
                };
                parent_task.set_verification_phase_context(phase_context);
            }

            // Include aggregation summary if fan-out was used
            if let Some(agg_summary) = parent_task
                .context
                .custom
                .get("aggregation_summary")
                .cloned()
            {
                parent_task.set_verification_aggregation_summary(agg_summary);
            }
        }

        // Embed the idempotency key in parent task context so that
        // IntentVerifierService::create_verification_task() can propagate it.
        parent_task.set_verification_idempotency_key(idem_key);
        if let Err(e) = self.task_repo.update(&parent_task).await {
            tracing::warn!(
                task_id = %task_id,
                error = %e,
                "WorkflowVerificationHandler: failed to persist idempotency key on parent task"
            );
        }

        // Extract goal_id from parent task context
        let goal_id = parent_task.goal_id();

        // Spawn the LLM verification in the background so we return within
        // the event reactor's 15s handler timeout.
        let intent_verifier = self.intent_verifier.clone();
        let task_repo = self.task_repo.clone();
        let event_bus = self.event_bus.clone();
        let verification_enabled = self.verification_enabled;
        let phase_name_owned = phase_name.clone();

        // intentional fire-and-forget: one-shot work
        tokio::spawn(async move {
            let (satisfied, summary) = match intent_verifier
                .verify_convergent_intent(
                    &parent_task,
                    goal_id,
                    retry_count,
                    None, // no overseer signals
                )
                .await
            {
                Ok(Some(result)) => {
                    use crate::domain::models::intent_verification::IntentSatisfaction;
                    let satisfied = result.satisfaction == IntentSatisfaction::Satisfied;
                    let summary = format!(
                        "Phase '{}' verification: {} (confidence: {:.2}, gaps: {})",
                        phase_name_owned,
                        result.satisfaction.as_str(),
                        result.confidence,
                        result.gaps.len(),
                    );
                    (satisfied, summary)
                }
                Ok(None) => {
                    tracing::info!(
                        task_id = %task_id,
                        "WorkflowVerificationHandler: no intent to verify, treating as satisfied"
                    );
                    (true, "No intent to verify — auto-passing".to_string())
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %task_id,
                        error = %e,
                        "WorkflowVerificationHandler: verification failed, treating as satisfied"
                    );
                    (true, format!("Verification infrastructure error: {}", e))
                }
            };

            // Feed result back to workflow engine (via TaskService for all mutations)
            let task_service = crate::services::task_service::TaskService::new(task_repo.clone());
            let engine = crate::services::workflow_engine::WorkflowEngine::new_with_config(
                task_repo,
                task_service,
                event_bus,
                verification_enabled,
            );
            if let Err(e) = engine
                .handle_verification_result(task_id, satisfied, &summary)
                .await
            {
                tracing::warn!(
                    task_id = %task_id,
                    "WorkflowVerificationHandler: handle_verification_result failed: {}",
                    e
                );
            }
        });

        Ok(Reaction::None)
    }
}

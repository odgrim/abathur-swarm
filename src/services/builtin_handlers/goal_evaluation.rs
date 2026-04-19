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
// GoalEvaluationHandler
// ============================================================================

/// Triggered by the "goal-evaluation" scheduled event (60s).
/// Observes task/memory state independently and emits signal events about
/// Filter tasks whose inferred domains overlap with a goal's applicability domains.
///
/// Universal goals (empty domains) match all tasks. Otherwise, each task's
/// domains are inferred via `GoalContextService::infer_task_domains` and
/// checked for overlap with the goal's domains.
fn filter_tasks_by_goal_domains<'a, G: GoalRepository>(
    tasks: &'a [Task],
    goal: &Goal,
) -> Vec<&'a Task> {
    let goal_domains = &goal.applicability_domains;
    tasks
        .iter()
        .filter(|t| {
            goal_domains.is_empty() || {
                let task_domains = GoalContextService::<G>::infer_task_domains(t);
                task_domains.iter().any(|d| goal_domains.contains(d))
            }
        })
        .collect()
}

/// goal progress. This is a read-only observer that never modifies goals,
/// tasks, or memories.
pub struct GoalEvaluationHandler<G: GoalRepository, T: TaskRepository> {
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
}

impl<G: GoalRepository, T: TaskRepository> GoalEvaluationHandler<G, T> {
    pub fn new(goal_repo: Arc<G>, task_repo: Arc<T>) -> Self {
        Self {
            goal_repo,
            task_repo,
        }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static, T: TaskRepository + 'static> EventHandler
    for GoalEvaluationHandler<G, T>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalEvaluationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "goal-evaluation"
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
        // Load all active goals
        let goals = self
            .goal_repo
            .get_active_with_constraints()
            .await
            .map_err(|e| format!("Failed to get active goals: {}", e))?;

        if goals.is_empty() {
            return Ok(Reaction::None);
        }

        // Get recent tasks (completed and failed)
        let completed = self
            .task_repo
            .list_by_status(TaskStatus::Complete)
            .await
            .map_err(|e| format!("Failed to list completed tasks: {}", e))?;
        let failed = self
            .task_repo
            .list_by_status(TaskStatus::Failed)
            .await
            .map_err(|e| format!("Failed to list failed tasks: {}", e))?;

        let mut new_events = Vec::new();

        for goal in &goals {
            // Find tasks whose inferred domains overlap with this goal's domains
            let relevant_completed = filter_tasks_by_goal_domains::<G>(&completed, goal);
            let relevant_failed = filter_tasks_by_goal_domains::<G>(&failed, goal);

            // Emit GoalIterationCompleted if there are completed tasks in matching domains
            if !relevant_completed.is_empty() {
                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Info,
                    category: EventCategory::Goal,
                    goal_id: Some(goal.id),
                    task_id: None,
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::GoalIterationCompleted {
                        goal_id: goal.id,
                        tasks_completed: relevant_completed.len(),
                    },
                });
            }

            // Check for constraint violations in failures
            for constraint in &goal.constraints {
                let violation_count = relevant_failed
                    .iter()
                    .filter(|t| {
                        // Check if failures relate to constraint violations
                        let hints = t.context.hints.join(" ").to_lowercase();
                        hints.contains(&constraint.name.to_lowercase())
                    })
                    .count();

                if violation_count > 0 {
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Warning,
                        category: EventCategory::Goal,
                        goal_id: Some(goal.id),
                        task_id: None,
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::GoalConstraintViolated {
                            goal_id: goal.id,
                            constraint_name: constraint.name.clone(),
                            violation: format!(
                                "{} task(s) failed with constraint-related errors",
                                violation_count
                            ),
                        },
                    });
                }
            }

            // Check for semantic drift: recurring failure patterns
            if relevant_failed.len() >= 3 {
                // Group failures by common error patterns
                let mut failure_hints: std::collections::HashMap<String, usize> =
                    std::collections::HashMap::new();
                for task in &relevant_failed {
                    for hint in &task.context.hints {
                        if hint.starts_with("Error:") {
                            let pattern = hint.chars().take(80).collect::<String>();
                            *failure_hints.entry(pattern).or_insert(0) += 1;
                        }
                    }
                }

                let recurring_gaps: Vec<String> = failure_hints
                    .into_iter()
                    .filter(|(_, count)| *count >= 2)
                    .map(|(pattern, _)| pattern)
                    .collect();

                if !recurring_gaps.is_empty() {
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Warning,
                        category: EventCategory::Goal,
                        goal_id: Some(goal.id),
                        task_id: None,
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::SemanticDriftDetected {
                            goal_id: goal.id,
                            recurring_gaps,
                            iterations: relevant_failed.len() as u32,
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

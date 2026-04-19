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
// GoalEvaluationTaskCreationHandler (Phase 4b)
// ============================================================================

/// Triggered by `SemanticDriftDetected` or `GoalConstraintViolated`.
/// Creates diagnostic/remediation tasks for recurring issues.
///
/// For `SemanticDriftDetected`, tasks are deduplicated by error pattern — not
/// per-goal.  When the same gap recurs across multiple goals, a single
/// investigation task is created and subsequent goals are recorded in its
/// description rather than spawning duplicate tasks.
pub struct GoalEvaluationTaskCreationHandler {
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    auto_create_diagnostic: bool,
    max_diagnostic_per_goal: u32,
    auto_create_remediation: bool,
    /// Tracks gap_hash → list of (goal_id, iterations) already folded into an
    /// investigation task.  Used to build an aggregated description on the first
    /// dispatch and to skip subsequent dispatches for the same error pattern.
    seen_gaps: tokio::sync::Mutex<std::collections::HashMap<u64, Vec<(uuid::Uuid, u32)>>>,
}

impl GoalEvaluationTaskCreationHandler {
    pub fn new(
        command_bus: Arc<crate::services::command_bus::CommandBus>,
        auto_create_diagnostic: bool,
        max_diagnostic_per_goal: u32,
        auto_create_remediation: bool,
    ) -> Self {
        Self {
            command_bus,
            auto_create_diagnostic,
            max_diagnostic_per_goal,
            auto_create_remediation,
            seen_gaps: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl EventHandler for GoalEvaluationTaskCreationHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalEvaluationTaskCreationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Goal])
                .payload_types(vec![
                    "SemanticDriftDetected".to_string(),
                    "GoalConstraintViolated".to_string(),
                ]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        use crate::domain::models::{TaskPriority, TaskSource};
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, TaskCommand,
        };

        match &event.payload {
            EventPayload::SemanticDriftDetected {
                goal_id,
                recurring_gaps,
                iterations,
            } if self.auto_create_diagnostic => {
                for (i, gap) in recurring_gaps.iter().enumerate() {
                    if i as u32 >= self.max_diagnostic_per_goal {
                        break;
                    }

                    let gap_hash_val = md5_lite(gap);

                    // Accumulate this goal into the per-gap tracker.  If we've
                    // already dispatched a task for this error pattern, skip —
                    // the idempotency key would dedup anyway, but skipping here
                    // avoids the command bus round-trip entirely.
                    let is_first = {
                        let mut seen = self.seen_gaps.lock().await;
                        let entry = seen.entry(gap_hash_val).or_default();
                        let first = entry.is_empty();
                        entry.push((*goal_id, *iterations));
                        first
                    };

                    if !is_first {
                        tracing::debug!(
                            gap_hash = %format!("{:x}", gap_hash_val),
                            goal_id = %goal_id,
                            "Skipping duplicate investigate task — already enqueued for this error pattern"
                        );
                        continue;
                    }

                    let gap_hash = format!("{:x}", gap_hash_val);
                    let idem_key = format!("drift:{}", gap_hash);
                    let title = format!("Investigate recurring gap: {}", truncate_str(gap, 60));
                    let description = format!(
                        "Recurring error pattern detected across convergence iterations.\n\n\
                         First observed in goal {} ({} iterations).\n\
                         Additional goals may be affected — query events with \
                         payload_type = 'SemanticDriftDetected' to identify the \
                         full set.\n\n\
                         Error pattern:\n{}",
                        goal_id, iterations, gap
                    );

                    let envelope = CommandEnvelope::new(
                        CommandSource::EventHandler(
                            "GoalEvaluationTaskCreationHandler".to_string(),
                        ),
                        DomainCommand::Task(TaskCommand::Submit {
                            title: Some(title),
                            description,
                            parent_id: None,
                            priority: TaskPriority::Normal,
                            agent_type: None,
                            depends_on: vec![],
                            context: Box::new(None),
                            idempotency_key: Some(idem_key),
                            source: TaskSource::System,
                            deadline: None,
                            task_type: None,
                            execution_mode: None,
                        }),
                    );

                    if let Err(e) = self.command_bus.dispatch(envelope).await {
                        tracing::warn!(
                            "GoalEvaluationTaskCreationHandler: failed to create diagnostic task: {}",
                            e
                        );
                    }
                }
            }
            EventPayload::GoalConstraintViolated {
                goal_id,
                constraint_name,
                violation,
            } if self.auto_create_remediation => {
                let idem_key = format!("remediate:{}:{}", goal_id, constraint_name);
                let title = format!("Remediate constraint violation: {}", constraint_name);
                let description = format!(
                    "Constraint '{}' violated for goal {}:\n\n{}",
                    constraint_name, goal_id, violation
                );

                let envelope = CommandEnvelope::new(
                    CommandSource::EventHandler("GoalEvaluationTaskCreationHandler".to_string()),
                    DomainCommand::Task(TaskCommand::Submit {
                        title: Some(title),
                        description,
                        parent_id: None,
                        priority: TaskPriority::High,
                        agent_type: None,
                        depends_on: vec![],
                        context: Box::new(None),
                        idempotency_key: Some(idem_key),
                        source: TaskSource::System,
                        deadline: None,
                        task_type: None,
                        execution_mode: None,
                    }),
                );

                if let Err(e) = self.command_bus.dispatch(envelope).await {
                    tracing::warn!(
                        "GoalEvaluationTaskCreationHandler: failed to create remediation task: {}",
                        e
                    );
                }
            }
            _ => {}
        }

        Ok(Reaction::None)
    }
}

/// Simple string hash for idempotency keys.
fn md5_lite(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Truncate a string to a given length with ellipsis.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max.saturating_sub(3);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

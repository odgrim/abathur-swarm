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
// EvolutionEvaluationHandler
// ============================================================================

/// Triggered by the "evolution-evaluation" scheduled event (120s).
/// Queries recently completed/failed tasks, computes per-agent-type success
/// rates, and emits EvolutionTriggered when refinement is warranted.
pub struct EvolutionEvaluationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
}

impl<T: TaskRepository> EvolutionEvaluationHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for EvolutionEvaluationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EvolutionEvaluationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "evolution-evaluation"
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
        use std::collections::HashMap;

        // Get recently completed and failed tasks
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

        // Compute per-agent-type success rates
        let mut agent_stats: HashMap<String, (u32, u32)> = HashMap::new(); // (success, total)

        for task in &completed {
            let agent = task.agent_type.as_deref().unwrap_or("unknown");
            let entry = agent_stats.entry(agent.to_string()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += 1;
        }

        for task in &failed {
            if task.retry_count >= task.max_retries {
                let agent = task.agent_type.as_deref().unwrap_or("unknown");
                let entry = agent_stats.entry(agent.to_string()).or_insert((0, 0));
                entry.1 += 1;
            }
        }

        let mut new_events = Vec::new();

        // Emit EvolutionTriggered for agents with low success rates
        for (agent_name, (successes, total)) in &agent_stats {
            if *total >= 5 {
                let success_rate = *successes as f64 / *total as f64;
                if success_rate < 0.6 {
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Info,
                        category: EventCategory::Agent,
                        goal_id: None,
                        task_id: None,
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::EvolutionTriggered {
                            template_name: agent_name.clone(),
                            trigger: format!(
                                "Low success rate: {:.0}% ({}/{})",
                                success_rate * 100.0,
                                successes,
                                total
                            ),
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

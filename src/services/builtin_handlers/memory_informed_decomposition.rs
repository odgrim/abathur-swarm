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
// MemoryInformedDecompositionHandler (Phase 3a)
// ============================================================================

/// Triggered by `MemoryStored` where tier is semantic and type is pattern.
/// Fires goal re-evaluation for goals with overlapping domains.
pub struct MemoryInformedDecompositionHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    cooldown_secs: u64,
    /// Track (goal_id, last_fired) to avoid duplicate evaluations.
    cooldowns: Arc<RwLock<std::collections::HashMap<uuid::Uuid, chrono::DateTime<chrono::Utc>>>>,
}

impl<G: GoalRepository> MemoryInformedDecompositionHandler<G> {
    pub fn new(goal_repo: Arc<G>, cooldown_secs: u64) -> Self {
        Self {
            goal_repo,
            cooldown_secs,
            cooldowns: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for MemoryInformedDecompositionHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "MemoryInformedDecompositionHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Memory])
                .payload_types(vec!["MemoryStored".to_string()]),
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
        let (memory_id, key, namespace, tier, memory_type) = match &event.payload {
            EventPayload::MemoryStored {
                memory_id,
                key,
                namespace,
                tier,
                memory_type,
            } => (
                *memory_id,
                key.clone(),
                namespace.clone(),
                tier.clone(),
                memory_type.clone(),
            ),
            _ => return Ok(Reaction::None),
        };

        // Only trigger for semantic tier + pattern type
        if tier != "semantic" || memory_type != "pattern" {
            return Ok(Reaction::None);
        }

        let now = chrono::Utc::now();
        let goals = self
            .goal_repo
            .get_active_with_constraints()
            .await
            .map_err(|e| format!("Failed to get active goals: {}", e))?;

        let mut new_events = Vec::new();
        let mut cooldowns = self.cooldowns.write().await;

        for goal in &goals {
            // Check if namespace overlaps with goal domains.
            // Universal goals (empty domains) match all namespaces.
            let overlaps = goal.applicability_domains.is_empty()
                || goal
                    .applicability_domains
                    .iter()
                    .any(|d| d.eq_ignore_ascii_case(&namespace));
            if !overlaps {
                continue;
            }

            // Check cooldown
            if let Some(last) = cooldowns.get(&goal.id)
                && (now - *last).num_seconds() < self.cooldown_secs as i64
            {
                continue;
            }

            cooldowns.insert(goal.id, now);

            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: now,
                severity: EventSeverity::Info,
                category: EventCategory::Memory,
                goal_id: Some(goal.id),
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::MemoryInformedGoal {
                    goal_id: goal.id,
                    memory_id,
                    memory_key: key.clone(),
                },
            });

            // Also emit a goal-evaluation trigger
            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: now,
                severity: EventSeverity::Debug,
                category: EventCategory::Scheduler,
                goal_id: Some(goal.id),
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::ScheduledEventFired {
                    schedule_id: uuid::Uuid::new_v4(),
                    name: "goal-evaluation".to_string(),
                },
            });
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

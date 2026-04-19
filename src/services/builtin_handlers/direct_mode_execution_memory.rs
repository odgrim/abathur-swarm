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
// DirectModeExecutionMemoryHandler
// ============================================================================

/// When any task execution is recorded (TaskExecutionRecorded event), store a
/// lightweight episodic memory entry so the classification heuristic can learn
/// which complexity levels benefit from convergence vs. direct execution.
///
/// This handler complements `ConvergenceMemoryHandler` (which stores rich
/// convergence-specific outcomes) by recording every task completion -- direct
/// mode or convergent -- in the `execution_history` namespace, keeping the
/// observations uniform for the bandit/heuristic.
///
/// Idempotent: uses `execution-record:{task_id}` as an idempotency key.
pub struct DirectModeExecutionMemoryHandler<M: MemoryRepository> {
    memory_repo: Arc<M>,
}

impl<M: MemoryRepository> DirectModeExecutionMemoryHandler<M> {
    pub fn new(memory_repo: Arc<M>) -> Self {
        Self { memory_repo }
    }
}

#[async_trait]
impl<M: MemoryRepository + 'static> EventHandler for DirectModeExecutionMemoryHandler<M> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "DirectModeExecutionMemoryHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskExecutionRecorded".to_string()]),
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
        let (task_id, execution_mode, complexity, succeeded, tokens_used) = match &event.payload {
            EventPayload::TaskExecutionRecorded {
                task_id,
                execution_mode,
                complexity,
                succeeded,
                tokens_used,
            } => (
                *task_id,
                execution_mode.clone(),
                complexity.clone(),
                *succeeded,
                *tokens_used,
            ),
            _ => return Ok(Reaction::None),
        };

        // Idempotency: check if we already stored a memory for this task execution
        let idempotency_key = format!("execution-record:{}", task_id);
        let existing = self
            .memory_repo
            .get_by_key(&idempotency_key, "execution_history")
            .await
            .map_err(|e| format!("Failed to check existing execution memory: {}", e))?;
        if existing.is_some() {
            return Ok(Reaction::None);
        }

        let outcome_str = if succeeded { "succeeded" } else { "failed" };

        // Build a structured summary for the memory content
        let content = format!(
            "Task execution recorded for {task_id}:\n\
             - execution_mode: {execution_mode}\n\
             - complexity: {complexity}\n\
             - outcome: {outcome_str}\n\
             - tokens_used: {tokens_used}",
        );

        // Choose memory type based on outcome
        let memory_type = if succeeded {
            crate::domain::models::MemoryType::Pattern
        } else {
            crate::domain::models::MemoryType::Error
        };

        let mut memory = crate::domain::models::Memory::episodic(idempotency_key.clone(), content)
            .with_namespace("execution_history")
            .with_type(memory_type)
            .with_source("task_execution")
            .with_task(task_id);

        // Add goal context if available
        if let Some(goal_id) = event.goal_id {
            memory = memory.with_goal(goal_id);
        }

        // Tag with mode, complexity, and outcome for future queries
        memory = memory
            .with_tag(format!("mode:{}", execution_mode))
            .with_tag(format!("complexity:{}", complexity))
            .with_tag(format!("outcome:{}", outcome_str));

        // Store custom metadata for machine consumption
        memory.metadata.custom.insert(
            "tokens_used".to_string(),
            serde_json::Value::Number(serde_json::Number::from(tokens_used)),
        );
        memory.metadata.custom.insert(
            "execution_mode".to_string(),
            serde_json::Value::String(execution_mode.clone()),
        );
        memory.metadata.custom.insert(
            "complexity".to_string(),
            serde_json::Value::String(complexity.clone()),
        );
        memory
            .metadata
            .custom
            .insert("succeeded".to_string(), serde_json::Value::Bool(succeeded));

        // Lower relevance than convergence memory -- this is for learning, not active use
        memory.metadata.relevance = 0.5;

        self.memory_repo
            .store(&memory)
            .await
            .map_err(|e| format!("Failed to store execution memory: {}", e))?;

        tracing::info!(
            task_id = %task_id,
            execution_mode = %execution_mode,
            complexity = %complexity,
            succeeded = succeeded,
            "Stored execution history memory for classification heuristic"
        );

        Ok(Reaction::None)
    }
}

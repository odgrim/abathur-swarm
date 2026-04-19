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
// TaskCompletionLearningHandler (Phase 4a)
// ============================================================================

/// Triggered by `TaskCompletedWithResult`. Extracts learning data from task
/// results and stores pattern memories for tasks that required retries.
pub struct TaskCompletionLearningHandler {
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    min_retries: u32,
    store_efficiency: bool,
}

impl TaskCompletionLearningHandler {
    pub fn new(
        command_bus: Arc<crate::services::command_bus::CommandBus>,
        min_retries: u32,
        store_efficiency: bool,
    ) -> Self {
        Self {
            command_bus,
            min_retries,
            store_efficiency,
        }
    }
}

#[async_trait]
impl EventHandler for TaskCompletionLearningHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskCompletionLearningHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskCompletedWithResult".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        _event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        use crate::domain::models::{MemoryTier, MemoryType};
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, MemoryCommand,
        };

        let result = match &_event.payload {
            EventPayload::TaskCompletedWithResult { result, .. } => result,
            _ => return Ok(Reaction::None),
        };

        // Store learning for tasks that required retries
        if result.retry_count >= self.min_retries {
            let error_summary = result.error.as_deref().unwrap_or("unknown");
            let key = format!(
                "task-learning:{}:{}",
                result.status,
                &error_summary.chars().take(40).collect::<String>()
            );

            let content = format!(
                "Task {} completed with status {} after {} retries in {}s. Error: {}",
                result.task_id,
                result.status,
                result.retry_count,
                result.duration_secs,
                error_summary
            );

            let envelope = CommandEnvelope::new(
                CommandSource::EventHandler("TaskCompletionLearningHandler".to_string()),
                DomainCommand::Memory(MemoryCommand::Store {
                    key,
                    content,
                    namespace: "task-learnings".to_string(),
                    tier: MemoryTier::Episodic,
                    memory_type: MemoryType::Pattern,
                    metadata: None,
                }),
            );

            if let Err(e) = self.command_bus.dispatch(envelope).await {
                tracing::warn!(
                    "TaskCompletionLearningHandler: failed to store learning: {}",
                    e
                );
            }
        }

        // Store efficiency pattern for fast completions
        if self.store_efficiency && result.retry_count == 0 && result.duration_secs < 60 {
            let key = format!("task-efficiency:{}", result.task_id);
            let content = format!(
                "Task {} completed efficiently: {}s, {} tokens",
                result.task_id, result.duration_secs, result.tokens_used
            );

            let envelope = CommandEnvelope::new(
                CommandSource::EventHandler("TaskCompletionLearningHandler".to_string()),
                DomainCommand::Memory(MemoryCommand::Store {
                    key,
                    content,
                    namespace: "task-learnings".to_string(),
                    tier: MemoryTier::Episodic,
                    memory_type: MemoryType::Pattern,
                    metadata: None,
                }),
            );

            if let Err(e) = self.command_bus.dispatch(envelope).await {
                tracing::debug!(
                    "TaskCompletionLearningHandler: failed to store efficiency pattern: {}",
                    e
                );
            }
        }

        Ok(Reaction::None)
    }
}

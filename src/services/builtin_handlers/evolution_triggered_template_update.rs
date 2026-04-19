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
// EvolutionTriggeredTemplateUpdateHandler (Phase 4c)
// ============================================================================

/// Triggered by `EvolutionTriggered`. If the agent template's success rate is
/// below 40%, submits a refinement task.
pub struct EvolutionTriggeredTemplateUpdateHandler {
    command_bus: Arc<crate::services::command_bus::CommandBus>,
}

impl EvolutionTriggeredTemplateUpdateHandler {
    pub fn new(command_bus: Arc<crate::services::command_bus::CommandBus>) -> Self {
        Self { command_bus }
    }
}

#[async_trait]
impl EventHandler for EvolutionTriggeredTemplateUpdateHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EvolutionTriggeredTemplateUpdateHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Agent])
                .payload_types(vec!["EvolutionTriggered".to_string()]),
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
        use crate::domain::models::{TaskPriority, TaskSource};
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, TaskCommand,
        };

        let (template_name, trigger) = match &event.payload {
            EventPayload::EvolutionTriggered {
                template_name,
                trigger,
            } => (template_name.clone(), trigger.clone()),
            _ => return Ok(Reaction::None),
        };

        // Parse success rate from the trigger string (e.g. "Low success rate: 40% (2/5)")
        let needs_refinement = trigger.contains("Low success rate");

        if !needs_refinement {
            return Ok(Reaction::None);
        }

        let title = format!("Refine agent template: {}", template_name);
        let description = format!(
            "Agent template '{}' triggered evolution: {}. Review and refine the template.",
            template_name, trigger
        );

        let envelope = CommandEnvelope::new(
            CommandSource::EventHandler("EvolutionTriggeredTemplateUpdateHandler".to_string()),
            DomainCommand::Task(TaskCommand::Submit {
                title: Some(title),
                description,
                parent_id: None,
                priority: TaskPriority::Normal,
                agent_type: None,
                depends_on: vec![],
                context: Box::new(None),
                idempotency_key: Some(format!("evolve:{}", template_name)),
                source: TaskSource::System,
                deadline: None,
                task_type: None,
                execution_mode: None,
            }),
        );

        if let Err(e) = self.command_bus.dispatch(envelope).await {
            tracing::warn!(
                "EvolutionTriggeredTemplateUpdateHandler: failed to submit refinement task: {}",
                e
            );
        }

        // Emit template status change
        let status_event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Agent,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::AgentTemplateStatusChanged {
                template_name,
                from_status: "active".to_string(),
                to_status: "under-review".to_string(),
            },
        };

        Ok(Reaction::EmitEvents(vec![status_event]))
    }
}

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
// EgressRoutingHandler (Adapter integration)
// ============================================================================

/// Routes task completion results to egress adapters. When a task completes
/// with a result containing an "egress" key, this handler deserializes the
/// [`EgressDirective`] and calls the appropriate egress adapter.
pub struct EgressRoutingHandler {
    adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
}

impl EgressRoutingHandler {
    /// Create a new egress routing handler.
    pub fn new(adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>) -> Self {
        Self { adapter_registry }
    }
}

#[async_trait]
impl EventHandler for EgressRoutingHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EgressRoutingHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskCompletedWithResult".to_string()]),
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
        let (task_id, result) = match &event.payload {
            EventPayload::TaskCompletedWithResult { task_id, result } => (*task_id, result),
            _ => return Ok(Reaction::None),
        };

        // Check if the result status contains egress routing info.
        // Convention: the result status field contains JSON with an "egress" key
        // when the completing agent wants to push results to an external system.
        let directive: crate::domain::models::adapter::EgressDirective = {
            // Try to parse the status field as JSON containing an egress directive
            let status_str = &result.status;
            match serde_json::from_str::<serde_json::Value>(status_str) {
                Ok(val) => {
                    if let Some(egress_val) = val.get("egress") {
                        match serde_json::from_value::<
                            crate::domain::models::adapter::EgressDirective,
                        >(egress_val.clone())
                        {
                            Ok(d) => d,
                            Err(_) => return Ok(Reaction::None),
                        }
                    } else {
                        return Ok(Reaction::None);
                    }
                }
                Err(_) => return Ok(Reaction::None),
            }
        };

        let adapter_name = &directive.adapter_name;
        let adapter = match self.adapter_registry.get_egress(adapter_name) {
            Some(a) => a,
            None => {
                tracing::warn!(
                    adapter = adapter_name.as_str(),
                    task_id = %task_id,
                    "Egress adapter not found"
                );
                let fail_event = crate::services::event_factory::make_event(
                    EventSeverity::Warning,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressFailed {
                        adapter_name: adapter_name.clone(),
                        task_id: Some(task_id),
                        error: format!("Adapter '{}' not found in registry", adapter_name),
                    },
                );
                return Ok(Reaction::EmitEvents(vec![fail_event]));
            }
        };

        let action_name = format!("{:?}", directive.action);

        match adapter.execute(&directive.action).await {
            Ok(egress_result) => {
                tracing::info!(
                    adapter = adapter_name.as_str(),
                    task_id = %task_id,
                    success = egress_result.success,
                    "Egress action completed"
                );
                let completed_event = crate::services::event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressCompleted {
                        adapter_name: adapter_name.clone(),
                        task_id,
                        action: action_name,
                        success: egress_result.success,
                    },
                );
                Ok(Reaction::EmitEvents(vec![completed_event]))
            }
            Err(e) => {
                tracing::warn!(
                    adapter = adapter_name.as_str(),
                    task_id = %task_id,
                    error = %e,
                    "Egress action failed"
                );
                let fail_event = crate::services::event_factory::make_event(
                    EventSeverity::Warning,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressFailed {
                        adapter_name: adapter_name.clone(),
                        task_id: Some(task_id),
                        error: e.to_string(),
                    },
                );
                Ok(Reaction::EmitEvents(vec![fail_event]))
            }
        }
    }
}

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
// A2APollHandler
// ============================================================================

/// Triggered by the "a2a-poll" scheduled event (15s).
/// Polls the A2A gateway for pending inbound delegations and submits tasks
/// through the CommandBus so they go through validation, dedup, and event journaling.
pub struct A2APollHandler {
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    a2a_gateway_url: String,
    consecutive_failures: AtomicU64,
}

impl A2APollHandler {
    pub fn new(
        command_bus: Arc<crate::services::command_bus::CommandBus>,
        a2a_gateway_url: String,
    ) -> Self {
        Self {
            command_bus,
            a2a_gateway_url,
            consecutive_failures: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl EventHandler for A2APollHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "A2APollHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "a2a-poll"
                    )
                })),
                ..Default::default()
            },
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
        use crate::domain::models::TaskPriority;
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, TaskCommand,
        };

        // Poll A2A gateway for pending inbound delegations
        let url = format!("{}/tasks/pending", self.a2a_gateway_url);
        let client = reqwest::Client::new();

        let response = match client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::warn!(
                    "A2APollHandler: gateway unreachable (consecutive failures: {}): {}",
                    failures,
                    e
                );
                if failures >= 3 {
                    let diagnostic = UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Warning,
                        category: EventCategory::Escalation,
                        goal_id: None,
                        task_id: None,
                        correlation_id: None,
                        source_process_id: None,
                        payload: EventPayload::HumanEscalationNeeded(HumanEscalationPayload {
                            goal_id: None,
                            task_id: None,
                            reason: format!(
                                "A2A gateway at {} has been unreachable for {} consecutive polls",
                                self.a2a_gateway_url, failures
                            ),
                            urgency: "medium".to_string(),
                            questions: vec![],
                            is_blocking: false,
                        }),
                    };
                    return Ok(Reaction::EmitEvents(vec![diagnostic]));
                }
                return Ok(Reaction::None);
            }
        };

        if !response.status().is_success() {
            let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::warn!(
                "A2APollHandler: gateway returned non-success status {} (consecutive failures: {})",
                response.status(),
                failures
            );
            return Ok(Reaction::None);
        }

        // Reset consecutive failure counter on success
        self.consecutive_failures.store(0, Ordering::Relaxed);

        let delegations: Vec<serde_json::Value> = match response.json().await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("A2APollHandler: failed to parse response: {}", e);
                return Ok(Reaction::None);
            }
        };

        for delegation in delegations {
            let title = delegation
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("A2A Delegated Task")
                .to_string();
            let description = delegation
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let envelope = CommandEnvelope::new(
                CommandSource::A2A("inbound-delegation".to_string()),
                DomainCommand::Task(TaskCommand::Submit {
                    title: Some(title.clone()),
                    description,
                    parent_id: None,
                    priority: TaskPriority::Normal,
                    agent_type: None,
                    depends_on: vec![],
                    context: Box::new(None),
                    idempotency_key: None,
                    source: crate::domain::models::TaskSource::System,
                    deadline: None,
                    task_type: None,
                    execution_mode: None,
                }),
            );

            if let Err(e) = self.command_bus.dispatch(envelope).await {
                tracing::warn!("A2APollHandler: failed to submit task '{}': {}", title, e);
            }
        }

        // Events are emitted by the CommandBus pipeline; no manual emission needed.
        Ok(Reaction::None)
    }
}

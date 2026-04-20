//! `SubsystemServices` — long-lived service instances always present
//! (non-Option). Holds the "core" services: audit log, circuit breaker,
//! evolution loop, restructure service, guardrails, and the event-bus
//! triple (bus + reactor + scheduler).
//!
//! Part of the T11 god-object decomposition (see
//! `specs/T11-swarm-orchestrator-decomposition.md`). Generic-free.

use std::sync::Arc;

use crate::services::{
    AuditLogService, CircuitBreakerService, EvolutionLoop,
    dag_restructure::DagRestructureService,
    event_bus::EventBus,
    event_reactor::EventReactor,
    event_scheduler::EventScheduler,
    guardrails::Guardrails,
};

/// Core services that are always wired up at construction (no `Option`),
/// shared across orchestrator subsystems. Does not own optional features —
/// those live in `AdvancedServices`.
// dead_code: introduced in T11 step 1; wired in step 2, fields used in step 7.
#[allow(dead_code)]
pub(super) struct SubsystemServices {
    pub(super) audit_log: Arc<AuditLogService>,
    pub(super) circuit_breaker: Arc<CircuitBreakerService>,
    pub(super) evolution_loop: Arc<EvolutionLoop>,
    pub(super) restructure_service: Arc<tokio::sync::Mutex<DagRestructureService>>,
    pub(super) guardrails: Arc<Guardrails>,

    pub(super) event_bus: Arc<EventBus>,
    pub(super) event_reactor: Arc<EventReactor>,
    pub(super) event_scheduler: Arc<EventScheduler>,
}

#[allow(dead_code)]
impl SubsystemServices {
    /// Construct subsystem services with default configurations for the
    /// non-injected ones. The event-bus triple is injected from the caller
    /// because it must be shared with other process-wide consumers (TUI,
    /// CLI, MCP servers).
    pub(super) fn new(
        event_bus: Arc<EventBus>,
        event_reactor: Arc<EventReactor>,
        event_scheduler: Arc<EventScheduler>,
    ) -> Self {
        Self {
            audit_log: Arc::new(AuditLogService::with_defaults()),
            circuit_breaker: Arc::new(CircuitBreakerService::with_defaults()),
            evolution_loop: Arc::new(EvolutionLoop::with_default_config()),
            restructure_service: Arc::new(tokio::sync::Mutex::new(
                DagRestructureService::with_defaults(),
            )),
            guardrails: Arc::new(Guardrails::with_defaults()),
            event_bus,
            event_reactor,
            event_scheduler,
        }
    }
}
